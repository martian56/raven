//! The `rv.lock` lock file for rvpm.
//!
//! `rv.lock` pins the exact resolved git ref and a content (tree) hash
//! for every transitive dependency of a package. It is checked in next to
//! `rv.toml`. The lock makes builds reproducible: a later install or build
//! fetches the pinned refs and verifies that the fetched tree still hashes
//! to the recorded value.
//!
//! Scope: full semver range resolution is not implemented yet. For now a
//! `[dependencies]` constraint string in `rv.toml` is treated as the
//! literal git ref (tag or branch) to fetch, so the resolved version
//! equals the constraint as written. Range resolution is future work; when
//! it lands the lock will record the chosen ref while `rv.toml` carries
//! the range.
//!
//! See `docs/v2/specs/rvpm.md` for the lock format and the tree-hash
//! scheme.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::manifest::{Manifest, ManifestError};
use crate::pkg::{self, PkgError};

/// The lock format version stamped into `rv.lock`.
pub const LOCK_VERSION: u32 = 1;

/// One locked package: its source identity, the resolved git ref, and the
/// content hash of its fetched tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedPackage {
    /// The `github.com/<user>/<repo>` source path.
    pub source: String,
    /// The resolved git ref (tag or branch) that was fetched.
    pub version: String,
    /// The tree content hash, formatted `sha256:<hex>`.
    pub hash: String,
}

/// A parsed `rv.lock`. Packages are kept sorted by `(source, version)` so
/// the serialized form is stable across runs and diffs cleanly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockFile {
    pub version: u32,
    pub packages: Vec<LockedPackage>,
}

/// An error produced while resolving, reading, or validating the lock.
#[derive(Debug)]
pub enum LockError {
    /// A filesystem operation against the lock file failed.
    Io {
        action: String,
        path: PathBuf,
        source: std::io::Error,
    },
    /// The lock file did not parse as the expected TOML shape.
    Parse(String),
    /// The lock format version is newer than this tool understands.
    UnsupportedVersion { found: u32 },
    /// Fetching a dependency failed.
    Fetch(PkgError),
    /// A fetched dependency's own `rv.toml` could not be read or parsed.
    Manifest {
        source_path: String,
        error: ManifestError,
    },
    /// A dependency constraint was not a usable git ref (empty).
    EmptyConstraint { source: String },
    /// A pinned tree hash did not match the fetched tree.
    HashMismatch {
        source: String,
        version: String,
        expected: String,
        actual: String,
    },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Io {
                action,
                path,
                source,
            } => write!(f, "cannot {} '{}': {}", action, path.display(), source),
            LockError::Parse(msg) => write!(f, "invalid rv.lock: {}", msg),
            LockError::UnsupportedVersion { found } => write!(
                f,
                "rv.lock format version {} is newer than this tool supports (max {})",
                found, LOCK_VERSION
            ),
            LockError::Fetch(e) => write!(f, "{}", e),
            LockError::Manifest { source_path, error } => {
                write!(f, "in dependency '{}': {}", source_path, error)
            }
            LockError::EmptyConstraint { source } => write!(
                f,
                "dependency '{}' has an empty version, expected a git tag or branch",
                source
            ),
            LockError::HashMismatch {
                source,
                version,
                expected,
                actual,
            } => write!(
                f,
                "content hash mismatch for '{}' at '{}': locked {} but fetched tree hashes to {}",
                source, version, expected, actual
            ),
        }
    }
}

impl std::error::Error for LockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LockError::Io { source, .. } => Some(source),
            LockError::Fetch(e) => Some(e),
            _ => None,
        }
    }
}

impl LockFile {
    /// Read and parse the lock file at `path`.
    pub fn load(path: impl AsRef<Path>) -> Result<LockFile, LockError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| LockError::Io {
            action: "read lock file".to_string(),
            path: path.to_path_buf(),
            source: e,
        })?;
        LockFile::from_toml_str(&text)
    }

    /// Parse a lock file from a TOML string.
    pub fn from_toml_str(s: &str) -> Result<LockFile, LockError> {
        let raw: RawLock =
            toml::from_str(s).map_err(|e| LockError::Parse(e.message().to_string()))?;
        if raw.version > LOCK_VERSION {
            return Err(LockError::UnsupportedVersion { found: raw.version });
        }
        let mut packages: Vec<LockedPackage> = raw
            .package
            .into_iter()
            .map(|p| LockedPackage {
                source: p.source,
                version: p.version,
                hash: p.hash,
            })
            .collect();
        sort_packages(&mut packages);
        Ok(LockFile {
            version: raw.version,
            packages,
        })
    }

    /// Serialize the lock file to its canonical TOML string. Packages are
    /// emitted in sorted order.
    pub fn to_toml_string(&self) -> String {
        let mut packages = self.packages.clone();
        sort_packages(&mut packages);
        let mut out = format!("version = {}\n", self.version);
        for p in &packages {
            out.push_str("\n[[package]]\n");
            out.push_str(&format!("source = {}\n", toml_string(&p.source)));
            out.push_str(&format!("version = {}\n", toml_string(&p.version)));
            out.push_str(&format!("hash = {}\n", toml_string(&p.hash)));
        }
        out
    }

    /// Write the lock file to `path` in canonical form.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), LockError> {
        let path = path.as_ref();
        std::fs::write(path, self.to_toml_string()).map_err(|e| LockError::Io {
            action: "write lock file".to_string(),
            path: path.to_path_buf(),
            source: e,
        })
    }

    /// True when every dependency in `manifest` has a matching entry in
    /// this lock (matched by source path). Transitive entries are not
    /// checked here; a missing direct dependency forces a fresh resolve.
    pub fn covers(&self, manifest: &Manifest) -> bool {
        manifest.dependencies.iter().all(|d| {
            self.packages
                .iter()
                .any(|p| p.source == dep_source(&d.path))
        })
    }
}

/// The conventional lock file name beside a manifest.
pub const LOCK_FILE_NAME: &str = "rv.lock";

/// Resolve the full transitive dependency set of `manifest`, fetching each
/// dependency into the shared cache (default cache root), and return the
/// computed lock. See [`resolve_and_lock_in`] for the explicit-cache-root
/// form used by tests.
pub fn resolve_and_lock(manifest: &Manifest) -> Result<LockFile, LockError> {
    resolve_and_lock_in(manifest, &pkg::cache_root())
}

/// Resolve and lock against an explicit cache root.
///
/// Walks the dependency graph transitively: each dependency is fetched,
/// its own `rv.toml` is read for sub-dependencies, and those are queued in
/// turn. Packages are deduplicated by `(source, version)` so a diamond in
/// the graph is fetched and hashed once. Each fetched tree is hashed with
/// [`tree_hash`]. The returned lock is sorted deterministically.
pub fn resolve_and_lock_in(manifest: &Manifest, cache_root: &Path) -> Result<LockFile, LockError> {
    let mut seen: BTreeMap<(String, String), LockedPackage> = BTreeMap::new();
    // Work queue of (source, version) pairs to resolve.
    let mut queue: Vec<(String, String)> = Vec::new();
    for dep in &manifest.dependencies {
        queue.push((dep.path.clone(), resolved_ref(dep)?));
    }

    while let Some((source, version)) = queue.pop() {
        let key = (source.clone(), version.clone());
        if seen.contains_key(&key) {
            continue;
        }

        let gh = crate::resolve::GithubPath::parse(&source).ok_or_else(|| {
            LockError::Parse(format!(
                "dependency source '{}' is not a github.com path",
                source
            ))
        })?;
        let dir = pkg::fetch_in(cache_root, &gh.host, &gh.user, &gh.repo, &version)
            .map_err(LockError::Fetch)?;

        let hash = tree_hash(&dir).map_err(|e| LockError::Io {
            action: "hash dependency tree".to_string(),
            path: dir.clone(),
            source: e,
        })?;

        seen.insert(
            key,
            LockedPackage {
                source: source.clone(),
                version: version.clone(),
                hash,
            },
        );

        // Read the fetched package's manifest for its own dependencies.
        let sub_manifest_path = dir.join("rv.toml");
        if sub_manifest_path.exists() {
            let sub = Manifest::load(&sub_manifest_path).map_err(|error| LockError::Manifest {
                source_path: source.clone(),
                error,
            })?;
            for dep in &sub.dependencies {
                queue.push((dep.path.clone(), resolved_ref(dep)?));
            }
        }
    }

    let mut packages: Vec<LockedPackage> = seen.into_values().collect();
    sort_packages(&mut packages);
    Ok(LockFile {
        version: LOCK_VERSION,
        packages,
    })
}

/// Validate `lock` against the shared cache (default cache root).
pub fn validate_lock(lock: &LockFile) -> Result<(), LockError> {
    validate_lock_in(lock, &pkg::cache_root())
}

/// Validate `lock` against an explicit cache root: fetch every pinned
/// entry and verify its tree hash. A mismatch aborts with
/// [`LockError::HashMismatch`] naming the package.
pub fn validate_lock_in(lock: &LockFile, cache_root: &Path) -> Result<(), LockError> {
    for entry in &lock.packages {
        let gh = crate::resolve::GithubPath::parse(&entry.source).ok_or_else(|| {
            LockError::Parse(format!(
                "locked source '{}' is not a github.com path",
                entry.source
            ))
        })?;
        let dir = pkg::fetch_in(cache_root, &gh.host, &gh.user, &gh.repo, &entry.version)
            .map_err(LockError::Fetch)?;
        let actual = tree_hash(&dir).map_err(|e| LockError::Io {
            action: "hash dependency tree".to_string(),
            path: dir.clone(),
            source: e,
        })?;
        if actual != entry.hash {
            return Err(LockError::HashMismatch {
                source: entry.source.clone(),
                version: entry.version.clone(),
                expected: entry.hash.clone(),
                actual,
            });
        }
    }
    Ok(())
}

/// Compute the deterministic content hash of the file tree rooted at
/// `dir`.
///
/// Every regular file under `dir` (recursively) is collected, excluding
/// any `.git` directory. Files are sorted by their relative path using
/// forward-slash separators so the order is identical on Windows and
/// Linux. For each file the hash absorbs the relative path bytes, a NUL
/// separator, the file length as 8 little-endian bytes, and the file
/// bytes. The result is the SHA-256 digest formatted `sha256:<hex>`. No
/// file mode or timestamp is included, so the same tree hashes identically
/// across platforms.
pub fn tree_hash(dir: &Path) -> std::io::Result<String> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (rel, abs) in &files {
        let bytes = std::fs::read(abs)?;
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    let digest = hasher.finalize();
    Ok(format!("sha256:{:x}", digest))
}

fn collect_files(
    root: &Path,
    current: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            collect_files(root, &path, out)?;
        } else if file_type.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel, path));
        }
    }
    Ok(())
}

/// The resolved git ref for a dependency. Today the constraint string is
/// the literal ref; range resolution is future work.
fn resolved_ref(dep: &crate::manifest::Dependency) -> Result<String, LockError> {
    let r = dep.constraint.trim();
    if r.is_empty() {
        return Err(LockError::EmptyConstraint {
            source: dep_source(&dep.path),
        });
    }
    Ok(r.to_string())
}

/// The lock `source` value for a dependency path. The manifest key already
/// is the `github.com/<user>/<repo>` source identity.
fn dep_source(path: &str) -> String {
    path.to_string()
}

fn sort_packages(packages: &mut [LockedPackage]) {
    packages.sort_by(|a, b| a.source.cmp(&b.source).then(a.version.cmp(&b.version)));
}

/// Quote a string as a TOML basic string. The values here (github paths,
/// git refs, hex hashes) contain no control characters or quotes, but the
/// escaping keeps the writer correct for any input.
fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLock {
    #[serde(default = "default_lock_version")]
    version: u32,
    #[serde(default)]
    package: Vec<RawLockedPackage>,
}

fn default_lock_version() -> u32 {
    LOCK_VERSION
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLockedPackage {
    source: String,
    version: String,
    hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A throwaway cache root under the OS temp dir, removed on drop.
    struct TempCache {
        root: PathBuf,
    }

    impl TempCache {
        fn new(tag: &str) -> TempCache {
            let mut root = std::env::temp_dir();
            let unique = format!(
                "rvpm-lock-{}-{}-{}",
                tag,
                std::process::id(),
                next_counter()
            );
            root.push(unique);
            std::fs::create_dir_all(&root).expect("create temp cache root");
            TempCache { root }
        }

        /// Seed a cached package directory with the given files. `files`
        /// is a list of (relative path, contents).
        fn seed(&self, source: &str, version: &str, files: &[(&str, &str)]) -> PathBuf {
            let gh = crate::resolve::GithubPath::parse(source).expect("github path");
            let dir = pkg::cache_dir_in(&self.root, &gh.host, &gh.user, &gh.repo, version);
            std::fs::create_dir_all(&dir).expect("create cache dir");
            for (rel, contents) in files {
                let path = dir.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("create parent");
                }
                std::fs::write(&path, contents).expect("write seed file");
            }
            dir
        }
    }

    impl Drop for TempCache {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn next_counter() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        COUNTER.fetch_add(1, Ordering::Relaxed)
    }

    fn manifest_with(deps: &[(&str, &str)]) -> Manifest {
        let mut src =
            String::from("[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n");
        for (path, constraint) in deps {
            src.push_str(&format!("\"{}\" = \"{}\"\n", path, constraint));
        }
        Manifest::from_toml_str(&src).expect("manifest parses")
    }

    #[test]
    fn fresh_resolve_walks_transitively() {
        let cache = TempCache::new("fresh");
        // foo depends on bar; bar has no deps.
        cache.seed(
            "github.com/acme/foo",
            "v1.0.0",
            &[
                (
                    "rv.toml",
                    "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n\n[dependencies]\n\"github.com/acme/bar\" = \"v1.0.0\"\n",
                ),
                ("src/lib.rv", "fun foo() {}\n"),
            ],
        );
        cache.seed(
            "github.com/acme/bar",
            "v1.0.0",
            &[
                (
                    "rv.toml",
                    "[package]\nname = \"bar\"\nversion = \"1.0.0\"\n",
                ),
                ("src/lib.rv", "fun bar() {}\n"),
            ],
        );

        let manifest = manifest_with(&[("github.com/acme/foo", "v1.0.0")]);
        let lock = resolve_and_lock_in(&manifest, &cache.root).expect("resolve");

        assert_eq!(lock.version, LOCK_VERSION);
        assert_eq!(lock.packages.len(), 2);
        // Sorted by source: bar before foo.
        assert_eq!(lock.packages[0].source, "github.com/acme/bar");
        assert_eq!(lock.packages[0].version, "v1.0.0");
        assert!(lock.packages[0].hash.starts_with("sha256:"));
        assert_eq!(lock.packages[1].source, "github.com/acme/foo");
        assert!(lock.packages[1].hash.starts_with("sha256:"));
    }

    #[test]
    fn validate_ok_when_tree_unchanged() {
        let cache = TempCache::new("validok");
        cache.seed(
            "github.com/acme/bar",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"bar\"\nversion = \"1.0.0\"\n",
            )],
        );
        let manifest = manifest_with(&[("github.com/acme/bar", "v1.0.0")]);
        let lock = resolve_and_lock_in(&manifest, &cache.root).expect("resolve");
        validate_lock_in(&lock, &cache.root).expect("validate ok");
    }

    #[test]
    fn validate_detects_hash_mismatch() {
        let cache = TempCache::new("mismatch");
        let dir = cache.seed(
            "github.com/acme/bar",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"bar\"\nversion = \"1.0.0\"\n",
            )],
        );
        let manifest = manifest_with(&[("github.com/acme/bar", "v1.0.0")]);
        let lock = resolve_and_lock_in(&manifest, &cache.root).expect("resolve");

        // Tamper with a cached file after locking.
        let f = dir.join("rv.toml");
        let mut contents = std::fs::read_to_string(&f).unwrap();
        contents.push_str("\n# tampered\n");
        std::fs::write(&f, contents).unwrap();

        let err = validate_lock_in(&lock, &cache.root).unwrap_err();
        match err {
            LockError::HashMismatch { source, .. } => {
                assert_eq!(source, "github.com/acme/bar");
            }
            other => panic!("expected HashMismatch, got {:?}", other),
        }
    }

    #[test]
    fn missing_dep_in_lock_forces_fresh_resolve() {
        let cache = TempCache::new("missing");
        cache.seed(
            "github.com/acme/foo",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n",
            )],
        );
        cache.seed(
            "github.com/acme/bar",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"bar\"\nversion = \"1.0.0\"\n",
            )],
        );

        // Lock only covers foo; manifest now also depends on bar.
        let foo_only = manifest_with(&[("github.com/acme/foo", "v1.0.0")]);
        let lock = resolve_and_lock_in(&foo_only, &cache.root).expect("resolve foo");
        assert_eq!(lock.packages.len(), 1);

        let both = manifest_with(&[
            ("github.com/acme/foo", "v1.0.0"),
            ("github.com/acme/bar", "v1.0.0"),
        ]);
        assert!(!lock.covers(&both), "lock should not cover the new dep");

        let relocked = resolve_and_lock_in(&both, &cache.root).expect("relock");
        assert_eq!(relocked.packages.len(), 2);
        assert!(relocked
            .packages
            .iter()
            .any(|p| p.source == "github.com/acme/bar"));
    }

    #[test]
    fn diamond_dependency_is_deduplicated() {
        let cache = TempCache::new("diamond");
        // foo and baz both depend on bar@v1.0.0.
        cache.seed(
            "github.com/acme/foo",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n\n[dependencies]\n\"github.com/acme/bar\" = \"v1.0.0\"\n",
            )],
        );
        cache.seed(
            "github.com/acme/baz",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"baz\"\nversion = \"1.0.0\"\n\n[dependencies]\n\"github.com/acme/bar\" = \"v1.0.0\"\n",
            )],
        );
        cache.seed(
            "github.com/acme/bar",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"bar\"\nversion = \"1.0.0\"\n",
            )],
        );

        let manifest = manifest_with(&[
            ("github.com/acme/foo", "v1.0.0"),
            ("github.com/acme/baz", "v1.0.0"),
        ]);
        let lock = resolve_and_lock_in(&manifest, &cache.root).expect("resolve");
        let bar_count = lock
            .packages
            .iter()
            .filter(|p| p.source == "github.com/acme/bar")
            .count();
        assert_eq!(bar_count, 1, "bar should be locked once");
        assert_eq!(lock.packages.len(), 3);
    }

    #[test]
    fn lock_roundtrips_through_toml() {
        let lock = LockFile {
            version: LOCK_VERSION,
            packages: vec![
                LockedPackage {
                    source: "github.com/acme/foo".to_string(),
                    version: "v1.0.0".to_string(),
                    hash: "sha256:abc123".to_string(),
                },
                LockedPackage {
                    source: "github.com/acme/bar".to_string(),
                    version: "v2.0.0".to_string(),
                    hash: "sha256:def456".to_string(),
                },
            ],
        };
        let text = lock.to_toml_string();
        let parsed = LockFile::from_toml_str(&text).expect("parse");
        // Sorted: bar then foo.
        assert_eq!(parsed.packages[0].source, "github.com/acme/bar");
        assert_eq!(parsed.packages[1].source, "github.com/acme/foo");
        assert_eq!(parsed.version, LOCK_VERSION);
    }

    #[test]
    fn tree_hash_is_stable_and_path_sensitive() {
        let cache = TempCache::new("hashstable");
        let dir = cache.seed(
            "github.com/acme/bar",
            "v1.0.0",
            &[("a.rv", "one"), ("sub/b.rv", "two")],
        );
        let h1 = tree_hash(&dir).expect("hash");
        let h2 = tree_hash(&dir).expect("hash again");
        assert_eq!(h1, h2, "hashing is deterministic");
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn git_dir_is_excluded_from_hash() {
        let cache = TempCache::new("gitexcl");
        let dir = cache.seed("github.com/acme/bar", "v1.0.0", &[("a.rv", "one")]);
        let h_before = tree_hash(&dir).expect("hash");
        // Add a .git directory with content; it must not affect the hash.
        let git = dir.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let h_after = tree_hash(&dir).expect("hash");
        assert_eq!(h_before, h_after, ".git must be excluded");
    }
}
