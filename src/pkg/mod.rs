//! Package fetching and the shared content cache for rvpm.
//!
//! A GitHub dependency `github.com/<user>/<repo>@<version>` is resolved
//! by cloning the named tag or branch into a cache shared across
//! projects. The cache lives at `<cache_root>/<host>/<user>/<repo>@<version>`.
//!
//! Version-constraint resolution (semver ranges) and the lock file are
//! out of scope here; see issue #83. This module takes an explicit
//! `version` string that is a git tag or branch and fetches it.
//!
//! See `docs/v2/specs/rvpm.md` for the cache layout and clone strategy.

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
    cache_root()
        .join(host)
        .join(user)
        .join(format!("{}@{}", repo, version))
}

/// Fetch `host/user/repo` at `version` into the shared cache and return
/// the cache directory.
///
/// If the cache directory already exists and is non-empty this is a
/// cache hit: the existing directory is returned without contacting the
/// remote. Otherwise the repository is cloned from
/// `https://<host>/<user>/<repo>` at the given tag or branch.
pub fn fetch(host: &str, user: &str, repo: &str, version: &str) -> Result<PathBuf, PkgError> {
    let dest = cache_dir(host, user, repo, version);
    if is_populated(&dest) {
        return Ok(dest);
    }
    let url = format!("https://{}/{}/{}", host, user, repo);
    clone_from(&url, version, &dest)?;
    Ok(dest)
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
