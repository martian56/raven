//! Package fetching and the shared content cache for rvpm.
//!
//! A GitHub dependency `github.com/<user>/<repo>@<version>` is resolved by
//! downloading the named tag or branch into a cache shared across projects.
//! The cache lives at `<cache_root>/<host>/<user>/<repo>@<version>`. A version
//! is fetched as a gzip tarball through codeload (one HTTP GET, no history),
//! falling back to a shallow `git clone` when that is unavailable.
//!
//! Version-constraint resolution (semver ranges) is out of scope here.
//! This module takes an explicit `version` string that is a git tag or
//! branch and fetches it. The lock file (`rv.lock`) is built on top in
//! `raven::lock`.
//!
//! See `docs/v2/specs/rvpm.md` for the cache layout and fetch strategy.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// An error produced while fetching a package or touching the cache.
#[derive(Debug)]
pub enum PkgError {
    /// A filesystem operation against the cache failed.
    Io {
        action: String,
        path: PathBuf,
        source: std::io::Error,
    },
    /// The `git` executable could not be launched.
    GitNotFound(std::io::Error),
    /// The requested tag or branch does not exist on the remote.
    MissingRef { reference: String, stderr: String },
    /// `git clone` failed for a reason other than a missing ref.
    CloneFailed {
        url: String,
        reference: String,
        stderr: String,
    },
    /// The cache destination contains a parent (`..`) component, which would
    /// place it outside the cache root. A defense-in-depth check at directory
    /// creation, independent of the upstream component validation.
    UnsafeCachePath { path: PathBuf },
}

impl fmt::Display for PkgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PkgError::Io {
                action,
                path,
                source,
            } => write!(f, "cannot {} '{}': {}", action, path.display(), source),
            PkgError::GitNotFound(source) => {
                write!(f, "git is not installed or not on PATH: {}", source)
            }
            PkgError::MissingRef { reference, stderr } => write!(
                f,
                "tag or branch '{}' was not found on the remote: {}",
                reference,
                stderr.trim()
            ),
            PkgError::CloneFailed {
                url,
                reference,
                stderr,
            } => write!(
                f,
                "git clone of '{}' at '{}' failed: {}",
                url,
                reference,
                stderr.trim()
            ),
            PkgError::UnsafeCachePath { path } => write!(
                f,
                "refusing to create cache directory outside the cache root: '{}'",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PkgError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PkgError::Io { source, .. } => Some(source),
            PkgError::GitNotFound(source) => Some(source),
            _ => None,
        }
    }
}

/// The root of the shared cache.
///
/// Honors the `RVPM_CACHE_DIR` override when set; this is the supported
/// way to redirect the cache (used by tests and for isolating
/// environments). Otherwise the default is `${HOME}/.rvpm/cache`, where
/// `HOME` is `$HOME` on Unix and `%USERPROFILE%` on Windows. If neither
/// is set the cache falls back to `.rvpm/cache` under the current
/// directory.
pub fn cache_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("RVPM_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".rvpm").join("cache")
}

/// The cache directory for one package version:
/// `<cache_root>/<host>/<user>/<repo>@<version>`.
pub fn cache_dir(host: &str, user: &str, repo: &str, version: &str) -> PathBuf {
    cache_dir_in(&cache_root(), host, user, repo, version)
}

/// The cache directory for one package version under an explicit cache
/// root. Threading the root explicitly lets callers (and tests) avoid the
/// global `RVPM_CACHE_DIR` environment variable and the races it brings.
pub fn cache_dir_in(root: &Path, host: &str, user: &str, repo: &str, version: &str) -> PathBuf {
    root.join(host)
        .join(user)
        .join(format!("{}@{}", repo, version))
}

/// Whether `s` is safe to use as a single path component under the package
/// cache. A package's user, repo, and version each become a directory name, so
/// each must be a single, in-tree segment: non-empty, not a current or parent
/// directory reference, and free of a path separator, a Windows drive/stream
/// colon, or a control character. This blocks a malicious dependency from
/// steering cache directory creation outside the cache root.
pub fn is_safe_cache_component(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':') || c.is_control())
}

/// The outcome of a fetch: where the package landed, and whether it was
/// already in the cache (no network) or freshly downloaded.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub dir: PathBuf,
    /// True when the cache already held this version, so nothing was
    /// downloaded.
    pub cached: bool,
}

/// Fetch `host/user/repo` at `version` into the shared cache.
///
/// If the cache directory already exists and is non-empty this is a
/// cache hit: the existing directory is returned without contacting the
/// remote. Otherwise the version is downloaded (see [`fetch_in`]).
pub fn fetch(host: &str, user: &str, repo: &str, version: &str) -> Result<Fetched, PkgError> {
    fetch_in(&cache_root(), host, user, repo, version)
}

/// Fetch `host/user/repo` at `version` into an explicit cache root.
///
/// A populated cache directory is returned as-is. Otherwise the version is
/// downloaded as a gzip tarball (a single HTTP GET, no git history), which
/// is faster than a clone. If the tarball path is unavailable (no `curl` or
/// `tar`, a non-github host, or a transient error) it falls back to a
/// shallow `git clone`, which also reports a genuinely missing ref.
///
/// The download lands in a sibling staging directory and is promoted into
/// the final cache path with a single atomic rename once it is complete, so
/// an interrupted fetch (a killed process, a crash mid-extract) never leaves
/// a partial directory that a later run would mistake for a complete cache
/// entry.
pub fn fetch_in(
    root: &Path,
    host: &str,
    user: &str,
    repo: &str,
    version: &str,
) -> Result<Fetched, PkgError> {
    let dest = cache_dir_in(root, host, user, repo, version);
    // Defense in depth: a `..` in any component would place the cache entry
    // outside the cache root. The path components are validated upstream, but
    // this guards the directory creation directly.
    if dest
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(PkgError::UnsafeCachePath { path: dest.clone() });
    }
    if is_populated(&dest) {
        return Ok(Fetched {
            dir: dest,
            cached: true,
        });
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| PkgError::Io {
            action: "create cache directory".to_string(),
            path: parent.to_path_buf(),
            source: e,
        })?;
    }

    // Stage into a sibling temp dir on the same filesystem so the promote is a
    // cheap atomic rename. Clear any leftover staging from a prior crash first.
    let staging = staging_dir(&dest);
    let _ = std::fs::remove_dir_all(&staging);
    let staged = download_tarball(host, user, repo, version, &staging).or_else(|_| {
        let _ = std::fs::remove_dir_all(&staging);
        let url = format!("https://{}/{}/{}", host, user, repo);
        clone_from(&url, version, &staging)
    });
    if let Err(e) = staged {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    promote(&staging, &dest)?;
    Ok(Fetched {
        dir: dest,
        cached: false,
    })
}

/// The sibling staging directory a fetch extracts into before promotion. It
/// sits beside the final entry (same parent, same filesystem) so the promote
/// can be a rename. The pid keeps concurrent processes from colliding.
fn staging_dir(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".rvpm-staging-{}", std::process::id()));
    dest.parent().unwrap_or(dest).join(name)
}

/// Move a completed staging directory into its final cache path with an atomic
/// rename. If another process populated `dest` first (a benign race), keep
/// theirs and discard the staging copy.
fn promote(staging: &Path, dest: &Path) -> Result<(), PkgError> {
    if is_populated(dest) {
        let _ = std::fs::remove_dir_all(staging);
        return Ok(());
    }
    // A non-populated `dest` may still exist as an empty leftover; clear it so
    // the rename does not fail on Windows, where rename onto an existing dir
    // errors.
    let _ = std::fs::remove_dir_all(dest);
    match std::fs::rename(staging, dest) {
        Ok(()) => Ok(()),
        // A racing process may have populated `dest` between our checks; if the
        // entry is good now, accept it and drop the staging copy.
        Err(_) if is_populated(dest) => {
            let _ = std::fs::remove_dir_all(staging);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(staging);
            Err(PkgError::Io {
                action: "promote cache entry".to_string(),
                path: dest.to_path_buf(),
                source: e,
            })
        }
    }
}

/// Download `github.com/user/repo` at `version` as a gzip tarball through
/// codeload and extract it into `dest`, stripping the version-prefixed top
/// directory the archive carries. Only `github.com` is served this way; any
/// failure (including a missing `curl`/`tar`) returns an error so the caller
/// can fall back to git. A partial `dest` is removed on failure.
fn download_tarball(
    host: &str,
    user: &str,
    repo: &str,
    version: &str,
    dest: &Path,
) -> Result<(), PkgError> {
    if host != "github.com" {
        return Err(PkgError::CloneFailed {
            url: host.to_string(),
            reference: version.to_string(),
            stderr: "tarball download is only available for github.com".to_string(),
        });
    }
    let url = format!(
        "https://codeload.github.com/{}/{}/tar.gz/{}",
        user, repo, version
    );
    let tmp = std::env::temp_dir().join(format!(
        "rvpm-{}-{}-{}-{}.tar.gz",
        user,
        repo,
        version.replace(['/', '\\', ':'], "_"),
        std::process::id()
    ));

    let dl = Command::new("curl")
        .args(["-fsSL", "--retry", "2", "-o"])
        .arg(&tmp)
        .arg(&url)
        .output()
        .map_err(PkgError::GitNotFound)?;
    if !dl.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(PkgError::CloneFailed {
            url,
            reference: version.to_string(),
            stderr: String::from_utf8_lossy(&dl.stderr).into_owned(),
        });
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| PkgError::Io {
            action: "create cache directory".to_string(),
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    std::fs::create_dir_all(dest).map_err(|e| PkgError::Io {
        action: "create cache entry".to_string(),
        path: dest.to_path_buf(),
        source: e,
    })?;

    let ex = Command::new("tar")
        .arg("-xzf")
        .arg(&tmp)
        .arg("-C")
        .arg(dest)
        .arg("--strip-components=1")
        .output();
    let _ = std::fs::remove_file(&tmp);
    let ex = match ex {
        Ok(o) => o,
        Err(e) => {
            let _ = std::fs::remove_dir_all(dest);
            return Err(PkgError::GitNotFound(e));
        }
    };
    if !ex.status.success() {
        let stderr = String::from_utf8_lossy(&ex.stderr).into_owned();
        let _ = std::fs::remove_dir_all(dest);
        return Err(PkgError::CloneFailed {
            url,
            reference: version.to_string(),
            stderr,
        });
    }
    Ok(())
}

/// True when `dir` exists and contains at least one entry.
fn is_populated(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => false,
    }
}

/// Clone `url` at `reference` (a tag or branch) into `dest`.
///
/// This is the network seam: callers pass `https://<host>/<user>/<repo>`
/// for the real path, while tests pass a local repository path so the
/// clone never touches the network. The clone is shallow
/// (`--depth 1 --branch <reference>`). On success the cloned `.git`
/// directory is removed so the cache holds only working-tree content; a
/// pinned tag or branch is never updated in place, so the history is not
/// needed.
pub fn clone_from(url: &str, reference: &str, dest: &Path) -> Result<(), PkgError> {
    // Defense in depth: never create a cache directory through a parent
    // reference, even if a component validation upstream were bypassed.
    if dest
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(PkgError::UnsafeCachePath {
            path: dest.to_path_buf(),
        });
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| PkgError::Io {
            action: "create cache directory".to_string(),
            path: parent.to_path_buf(),
            source: e,
        })?;
    }

    let output = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--branch")
        .arg(reference)
        .arg(url)
        .arg(dest)
        .output()
        .map_err(PkgError::GitNotFound)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        // git reports a missing tag or branch with this phrasing on the
        // clone path. Surface it as MissingRef so callers can be specific.
        if stderr.contains("Remote branch")
            || stderr.contains("not found in upstream")
            || stderr.contains("does not exist")
        {
            return Err(PkgError::MissingRef {
                reference: reference.to_string(),
                stderr,
            });
        }
        return Err(PkgError::CloneFailed {
            url: url.to_string(),
            reference: reference.to_string(),
            stderr,
        });
    }

    let git_dir = dest.join(".git");
    if git_dir.exists() {
        std::fs::remove_dir_all(&git_dir).map_err(|e| PkgError::Io {
            action: "remove .git from cache entry".to_string(),
            path: git_dir,
            source: e,
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn safe_cache_component_accepts_real_names_and_rejects_escapes() {
        // Real GitHub users, repos, and version refs, including a leading-dot
        // directory name (a normal segment, unlike `.`/`..`).
        for ok in [
            "martian56",
            "raven-http",
            "raven.rs",
            "v1.2.3",
            "v1.0.0-rc.1",
            "a_b",
            ".config",
        ] {
            assert!(is_safe_cache_component(ok), "should accept `{ok}`");
        }
        // Empty, current/parent refs, separators, a drive colon, and control
        // characters cannot be a single in-tree segment.
        for bad in [
            "", ".", "..", "a/b", "a\\b", "C:", "../x", "a\nb", "a\u{0}b",
        ] {
            assert!(!is_safe_cache_component(bad), "should reject `{bad:?}`");
        }
    }

    #[test]
    fn clone_from_refuses_a_parent_reference_in_the_dest() {
        let dest = Path::new("cache/github.com/../../escape@v1");
        let err = clone_from("https://example.invalid", "v1", dest)
            .expect_err("must reject a `..` cache path");
        assert!(
            matches!(err, PkgError::UnsafeCachePath { .. }),
            "got {err:?}"
        );
    }

    fn unique_temp(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let mut root = std::env::temp_dir();
        root.push(format!(
            "rvpm-pkg-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        root
    }

    #[test]
    fn cache_dir_layout() {
        std::env::set_var("RVPM_CACHE_DIR", "/tmp/rvpm-test-root");
        let dir = cache_dir("github.com", "acme", "json", "v1.0.0");
        std::env::remove_var("RVPM_CACHE_DIR");
        assert!(dir.ends_with("github.com/acme/json@v1.0.0"));
    }

    #[test]
    fn promote_moves_staging_into_place() {
        let root = unique_temp("promote");
        let dest = root.join("github.com/acme/foo@v1.0.0");
        let staging = staging_dir(&dest);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("lib.rv"), "fun foo() {}\n").unwrap();

        promote(&staging, &dest).expect("promote");

        assert!(is_populated(&dest), "dest is populated after promote");
        assert!(!staging.exists(), "staging is consumed by the rename");
        assert_eq!(
            std::fs::read_to_string(dest.join("lib.rv")).unwrap(),
            "fun foo() {}\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn promote_keeps_existing_entry_on_race() {
        let root = unique_temp("promoterace");
        let dest = root.join("github.com/acme/foo@v1.0.0");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("winner.rv"), "winner\n").unwrap();
        let staging = staging_dir(&dest);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("loser.rv"), "loser\n").unwrap();

        promote(&staging, &dest).expect("promote");

        assert!(dest.join("winner.rv").exists(), "existing entry is kept");
        assert!(
            !dest.join("loser.rv").exists(),
            "staging copy is discarded on a race"
        );
        assert!(!staging.exists(), "staging is cleaned up");
        let _ = std::fs::remove_dir_all(&root);
    }
}
