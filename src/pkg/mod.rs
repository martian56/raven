//! Package fetching and the shared content cache for rvpm.
//!
//! A GitHub dependency `github.com/<user>/<repo>@<version>` is resolved by
//! downloading the named tag or branch into a cache shared across projects.
//! The cache lives at `<cache_root>/<host>/<user>/<repo>@<version>`. A version
//! is fetched as a gzip tarball through codeload (one HTTP GET, no history),
//! falling back to a shallow `git clone` when that is unavailable. The hash of
//! each version is recorded in a `<repo>@<version>.rvpm-hash` sidecar beside
//! its directory so a warm install need not re-hash unchanged trees.
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
pub fn fetch_in(
    root: &Path,
    host: &str,
    user: &str,
    repo: &str,
    version: &str,
) -> Result<Fetched, PkgError> {
    let dest = cache_dir_in(root, host, user, repo, version);
    if is_populated(&dest) {
        return Ok(Fetched {
            dir: dest,
            cached: true,
        });
    }
    if download_tarball(host, user, repo, version, &dest).is_err() {
        // Clear any partial extraction, then fall back to a git clone.
        let _ = std::fs::remove_dir_all(&dest);
        let url = format!("https://{}/{}/{}", host, user, repo);
        clone_from(&url, version, &dest)?;
    }
    Ok(Fetched {
        dir: dest,
        cached: false,
    })
}

/// The sidecar file that caches a version's tree hash, kept beside (not
/// inside) the version directory so it never affects the tree hash itself.
pub fn hash_sidecar(version_dir: &Path) -> PathBuf {
    let mut name = version_dir.file_name().unwrap_or_default().to_os_string();
    name.push(".rvpm-hash");
    version_dir.parent().unwrap_or(version_dir).join(name)
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

    #[test]
    fn cache_dir_layout() {
        std::env::set_var("RVPM_CACHE_DIR", "/tmp/rvpm-test-root");
        let dir = cache_dir("github.com", "acme", "json", "v1.0.0");
        std::env::remove_var("RVPM_CACHE_DIR");
        assert!(dir.ends_with("github.com/acme/json@v1.0.0"));
    }
}
