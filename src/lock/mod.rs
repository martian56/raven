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

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::manifest::{Manifest, ManifestError};
use crate::pkg::{self, PkgError};

/// A callback invoked once per package as it is fetched during resolution or
/// validation, with the package source, version, and whether it was served
/// from the cache (no network). Set by [`set_fetch_observer`]; rvpm uses it
/// to print live progress.
type FetchObserver = Box<dyn Fn(&str, &str, bool) + Send + Sync>;

static FETCH_OBSERVER: Mutex<Option<FetchObserver>> = Mutex::new(None);

/// Install a process-wide fetch observer, or clear it with `None`. The
/// observer is called from worker threads, so it must be `Send + Sync`; calls
/// are serialized through a lock, so it does not need its own.
pub fn set_fetch_observer(observer: Option<FetchObserver>) {
    if let Ok(mut guard) = FETCH_OBSERVER.lock() {
        *guard = observer;
    }
}

fn notify_fetch(source: &str, version: &str, cached: bool) {
    if let Ok(guard) = FETCH_OBSERVER.lock() {
        if let Some(observer) = guard.as_ref() {
            observer(source, version, cached);
        }
    }
}

/// The most concurrent fetches to run at once. Fetching is network-bound, so
/// this exceeds the core count, but stays modest to avoid hammering the host.
fn max_fetch_workers() -> usize {
    12
}

/// Map `f` over `items` with bounded concurrency, preserving input order. A
/// single item runs inline; the rest fan out across scoped worker threads that
/// pull from a shared index.
fn par_map<T, R>(items: Vec<T>, f: impl Fn(T) -> R + Sync) -> Vec<R>
where
    T: Send,
    R: Send,
{
    let n = items.len();
    if n <= 1 {
        return items.into_iter().map(f).collect();
    }
    let workers = n.min(max_fetch_workers());
    let slots: Vec<Mutex<Option<T>>> = items.into_iter().map(|t| Mutex::new(Some(t))).collect();
    let results: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let f = &f;
    let slots = &slots;
    let results = &results;
    let next = &next;
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let item = slots[i].lock().unwrap().take().unwrap();
                let r = f(item);
                *results[i].lock().unwrap() = Some(r);
            });
        }
    });
    results
        .iter()
        .map(|m| m.lock().unwrap().take().unwrap())
        .collect()
}

/// Hash a fetched tree, reusing a persisted hash when the tree is unchanged so
/// a warm install does not re-read and re-hash every file.
///
/// The sidecar stores the content hash plus a cheap metadata signature (file
/// count, total bytes, newest mtime). When the recomputed signature matches,
/// the stored hash is trusted; when it differs, or no sidecar exists, the full
/// content hash is recomputed and the sidecar refreshed. The signature is a
/// metadata-only walk (no file reads), so it stays fast while still catching an
/// edited cache file, which changes its size or mtime.
fn hash_with_cache(dir: &Path) -> Result<String, LockError> {
    let to_io = |e: std::io::Error| LockError::Io {
        action: "hash dependency tree".to_string(),
        path: dir.to_path_buf(),
        source: e,
    };
    let signature = tree_signature(dir).map_err(to_io)?;
    let sidecar = pkg::hash_sidecar(dir);
    if let Ok(content) = std::fs::read_to_string(&sidecar) {
        let mut lines = content.lines();
        let stored_hash = lines.next().unwrap_or("").trim();
        let stored_sig = lines.next().unwrap_or("").trim();
        if stored_hash.starts_with("sha256:") && stored_sig == signature {
            return Ok(stored_hash.to_string());
        }
    }
    let hash = tree_hash(dir).map_err(to_io)?;
    let _ = std::fs::write(&sidecar, format!("{}\n{}\n", hash, signature));
    Ok(hash)
}

/// A cheap fingerprint of a tree from file metadata alone (no content reads):
/// file count, total bytes, and the newest mtime, formatted as one line. Any
/// edit to a cached file changes its size or mtime, so it changes the
/// signature. The `.git` directory is skipped, matching [`tree_hash`].
fn tree_signature(dir: &Path) -> std::io::Result<String> {
    let mut count: u64 = 0;
    let mut bytes: u64 = 0;
    let mut newest: u128 = 0;
    signature_walk(dir, &mut count, &mut bytes, &mut newest)?;
    Ok(format!("{} {} {}", count, bytes, newest))
}

fn signature_walk(
    dir: &Path,
    count: &mut u64,
    bytes: &mut u64,
    newest: &mut u128,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            signature_walk(&entry.path(), count, bytes, newest)?;
        } else {
            *count += 1;
            if let Ok(meta) = entry.metadata() {
                *bytes += meta.len();
                if let Ok(modified) = meta.modified() {
                    if let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH) {
                        *newest = (*newest).max(since.as_nanos());
                    }
                }
            }
        }
    }
    Ok(())
}

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
    /// A dependency constraint held a path separator or `..`, which could
    /// escape the cache directory when used as a directory name.
    InvalidConstraint { source: String, value: String },
    /// A pinned tree hash did not match the fetched tree.
    HashMismatch {
        source: String,
        version: String,
        expected: String,
        actual: String,
    },
    /// The dependency graph pins the same package source to two or more
    /// different versions. Resolution indexes packages by source, so a single
    /// version must win per source; rather than silently pick one (and compile
    /// a dependent against the wrong code), the conflict is reported.
    ConflictingVersions {
        source: String,
        versions: Vec<String>,
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
            LockError::InvalidConstraint { source, value } => write!(
                f,
                "dependency '{}' has an invalid version '{}': a version may not contain a path separator or '..'",
                source, value
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
            LockError::ConflictingVersions { source, versions } => write!(
                f,
                "dependency '{}' is required at conflicting versions ({}); a single version must be selected per package",
                source,
                versions.join(", ")
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
        // Drop exact-duplicate entries. The sort puts identical packages
        // adjacent, so `dedup` collapses a lock that repeats the same
        // `(source, version, hash)` block. Without this a duplicated entry is
        // gathered twice during a build, so a dependency's FFI C sources are
        // compiled twice and the linker fails with duplicate definitions. A
        // genuine conflict (one source at two versions) survives and is caught
        // by [`conflicting_versions`].
        packages.dedup();
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
            let source = dep_source(&d.path);
            // Match both the source and the requested version, so editing a
            // dependency's version in rv.toml invalidates the lock and forces a
            // re-resolve instead of silently reusing the old pinned version.
            match resolved_ref(d) {
                Ok(version) => self
                    .packages
                    .iter()
                    .any(|p| p.source == source && p.version == version),
                Err(_) => false,
            }
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
    let mut queued: BTreeSet<(String, String)> = BTreeSet::new();
    let mut wave: Vec<(String, String)> = Vec::new();
    for dep in &manifest.dependencies {
        let key = (dep.path.clone(), resolved_ref(dep)?);
        if queued.insert(key.clone()) {
            wave.push(key);
        }
    }

    // Fetch each level of the graph concurrently, then queue the dependencies
    // discovered from the fetched manifests as the next level.
    while !wave.is_empty() {
        let results = par_map(std::mem::take(&mut wave), |(source, version)| {
            fetch_and_read(cache_root, &source, &version)
        });
        for result in results {
            let (package, sub_deps) = result?;
            let key = (package.source.clone(), package.version.clone());
            seen.insert(key, package);
            for dep in sub_deps {
                if !seen.contains_key(&dep) && queued.insert(dep.clone()) {
                    wave.push(dep);
                }
            }
        }
    }

    let mut packages: Vec<LockedPackage> = seen.into_values().collect();
    sort_packages(&mut packages);
    if let Some((source, versions)) = conflicting_versions(&packages) {
        return Err(LockError::ConflictingVersions { source, versions });
    }
    Ok(LockFile {
        version: LOCK_VERSION,
        packages,
    })
}

/// If any package source appears with two or more distinct versions, return
/// that source and its conflicting versions (sorted, deduplicated). Import
/// resolution keys packages by source, so a source must resolve to exactly one
/// version; a conflict is reported rather than silently collapsed.
fn conflicting_versions(packages: &[LockedPackage]) -> Option<(String, Vec<String>)> {
    // A GitHub owner and repository path is case-insensitive, so `acme/Demo` and
    // `acme/demo` name the same repository. Group by the lowercased source so two
    // casing variants at different versions are caught as one conflict rather
    // than sneaking past as two separate sources. A representative original
    // spelling is kept for the message.
    let mut by_source: BTreeMap<String, (String, BTreeSet<String>)> = BTreeMap::new();
    for p in packages {
        let entry = by_source
            .entry(p.source.to_ascii_lowercase())
            .or_insert_with(|| (p.source.clone(), BTreeSet::new()));
        entry.1.insert(p.version.clone());
    }
    by_source
        .into_values()
        .find(|(_, vs)| vs.len() > 1)
        .map(|(s, vs)| (s, vs.into_iter().collect()))
}

/// Read the direct dependencies a cached package declares in its own
/// `rv.toml`, as `(source, version)` pairs. The package is fetched into the
/// cache if absent. A package with no `rv.toml` (or no `[dependencies]`)
/// contributes nothing.
pub fn cached_subdeps(
    cache_root: &Path,
    source: &str,
    version: &str,
) -> Result<Vec<(String, String)>, LockError> {
    let gh = crate::resolve::GithubPath::parse(source).ok_or_else(|| {
        LockError::Parse(format!(
            "dependency source '{}' is not a github.com path",
            source
        ))
    })?;
    let fetched = pkg::fetch_in(cache_root, &gh.host, &gh.user, &gh.repo, version)
        .map_err(LockError::Fetch)?;
    let manifest_path = fetched.dir.join("rv.toml");
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }
    let sub = Manifest::load(&manifest_path).map_err(|error| LockError::Manifest {
        source_path: source.to_string(),
        error,
    })?;
    let mut deps = Vec::with_capacity(sub.dependencies.len());
    for dep in &sub.dependencies {
        deps.push((dep.path.clone(), resolved_ref(dep)?));
    }
    Ok(deps)
}

/// Whether `lock` contains the complete transitive graph: every dependency
/// declared by a locked package's own `rv.toml` is itself a locked entry.
/// Reads each locked package's cached manifest. Used to detect a lock that
/// lists a package but omits the packages it pulls in.
pub fn lock_covers_transitive(lock: &LockFile, cache_root: &Path) -> Result<bool, LockError> {
    let have: BTreeSet<(String, String)> = lock
        .packages
        .iter()
        .map(|p| (p.source.clone(), p.version.clone()))
        .collect();
    for p in &lock.packages {
        for dep in cached_subdeps(cache_root, &p.source, &p.version)? {
            if !have.contains(&dep) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Fetch one package, report it to the observer, hash it (reusing a persisted
/// hash when present), and read its direct dependencies from the fetched
/// manifest.
fn fetch_and_read(
    cache_root: &Path,
    source: &str,
    version: &str,
) -> Result<(LockedPackage, Vec<(String, String)>), LockError> {
    let gh = crate::resolve::GithubPath::parse(source).ok_or_else(|| {
        LockError::Parse(format!(
            "dependency source '{}' is not a github.com path",
            source
        ))
    })?;
    let fetched = pkg::fetch_in(cache_root, &gh.host, &gh.user, &gh.repo, version)
        .map_err(LockError::Fetch)?;
    notify_fetch(source, version, fetched.cached);
    let hash = hash_with_cache(&fetched.dir)?;

    let mut sub_deps = Vec::new();
    let sub_manifest_path = fetched.dir.join("rv.toml");
    if sub_manifest_path.exists() {
        let sub = Manifest::load(&sub_manifest_path).map_err(|error| LockError::Manifest {
            source_path: source.to_string(),
            error,
        })?;
        for dep in &sub.dependencies {
            sub_deps.push((dep.path.clone(), resolved_ref(dep)?));
        }
    }
    Ok((
        LockedPackage {
            source: source.to_string(),
            version: version.to_string(),
            hash,
        },
        sub_deps,
    ))
}

/// Validate `lock` against the shared cache (default cache root).
pub fn validate_lock(lock: &LockFile) -> Result<(), LockError> {
    validate_lock_in(lock, &pkg::cache_root())
}

/// Validate `lock` against an explicit cache root: fetch every pinned
/// entry and verify its tree hash. A mismatch aborts with
/// [`LockError::HashMismatch`] naming the package.
pub fn validate_lock_in(lock: &LockFile, cache_root: &Path) -> Result<(), LockError> {
    if let Some((source, versions)) = conflicting_versions(&lock.packages) {
        return Err(LockError::ConflictingVersions { source, versions });
    }
    let results = par_map(lock.packages.clone(), |entry| {
        validate_one(cache_root, &entry)
    });
    for result in results {
        result?;
    }
    Ok(())
}

/// Fetch one locked package, report it, and verify its tree hash against the
/// lock. A persisted hash is reused when present.
fn validate_one(cache_root: &Path, entry: &LockedPackage) -> Result<(), LockError> {
    let gh = crate::resolve::GithubPath::parse(&entry.source).ok_or_else(|| {
        LockError::Parse(format!(
            "locked source '{}' is not a github.com path",
            entry.source
        ))
    })?;
    let fetched = pkg::fetch_in(cache_root, &gh.host, &gh.user, &gh.repo, &entry.version)
        .map_err(LockError::Fetch)?;
    notify_fetch(&entry.source, &entry.version, fetched.cached);
    // Validation is the integrity boundary, so always re-read and re-hash the
    // tree content rather than trusting the sidecar's metadata-signature
    // shortcut. The signature catches an honest edit (it changes the file size
    // or mtime), but an attacker who controls the cache can tamper a file and
    // rewrite the sidecar to reassert the old hash; only re-hashing the content
    // catches that.
    let actual = tree_hash(&fetched.dir).map_err(|e| LockError::Io {
        action: "hash dependency tree".to_string(),
        path: fetched.dir.clone(),
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
    Ok(())
}

/// Compute the deterministic content hash of the file tree rooted at
/// `dir`.
///
/// Every regular file and symlink under `dir` (recursively) is collected,
/// excluding any `.git` directory. Entries are sorted by relative path so the
/// order is identical across runs and platforms. For each entry the hash
/// absorbs the relative path one component at a time, then a kind discriminator
/// and the file bytes or the symlink target. Every variable-length field (a
/// path component, a symlink target, the file content) is length-prefixed, and
/// each component is tagged by whether it is valid UTF-8: a valid component
/// absorbs its UTF-8 bytes (identical across platforms), an invalid one absorbs
/// its native OS bytes (lossless, so two distinct non-Unicode names never
/// collide the way a lossy conversion would). The result is the SHA-256 digest
/// formatted `sha256:<hex>`. No file mode or timestamp is included.
pub fn tree_hash(dir: &Path) -> std::io::Result<String> {
    let mut files: Vec<(String, PathBuf, bool)> = Vec::new();
    collect_files(dir, dir, &mut files)?;
    // Sort by the (lossy) relative path for a cross-platform-stable order, with
    // the lossless absolute path as a tiebreaker so two names that share a lossy
    // form still order deterministically.
    files.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut hasher = Sha256::new();
    for (_rel, abs, is_symlink) in &files {
        let relative = abs.strip_prefix(dir).unwrap_or(abs);
        let components: Vec<_> = relative.components().collect();
        hasher.update((components.len() as u64).to_le_bytes());
        for component in components {
            absorb_os_str(&mut hasher, component.as_os_str());
        }
        if *is_symlink {
            // Record the link's target, length-prefixed, so a symlink is not
            // silently dropped and its bytes cannot run into the next entry.
            let target = std::fs::read_link(abs)?;
            hasher.update(b"L");
            absorb_os_str(&mut hasher, target.as_os_str());
        } else {
            let bytes = std::fs::read(abs)?;
            hasher.update(b"F");
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
    }
    let digest = hasher.finalize();
    Ok(format!("sha256:{:x}", digest))
}

/// Absorb a single `OsStr` into the hash losslessly and unambiguously: a tag
/// byte for whether it is valid UTF-8, the byte length, then the bytes. A valid
/// component uses its UTF-8 bytes so the hash is identical across platforms; an
/// invalid one uses its native OS bytes so distinct non-Unicode names stay
/// distinct instead of collapsing to a replacement character.
fn absorb_os_str(hasher: &mut Sha256, os: &std::ffi::OsStr) {
    match os.to_str() {
        Some(s) => {
            hasher.update([0u8]);
            hasher.update((s.len() as u64).to_le_bytes());
            hasher.update(s.as_bytes());
        }
        None => {
            let bytes = os_native_bytes(os);
            hasher.update([1u8]);
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
    }
}

/// The native byte encoding of an `OsStr` (UTF-8-ish bytes on Unix, the
/// little-endian UTF-16 code units on Windows). Only reached for a component
/// that is not valid Unicode, to keep distinct names distinct in the hash.
#[cfg(unix)]
fn os_native_bytes(os: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    os.as_bytes().to_vec()
}

#[cfg(windows)]
fn os_native_bytes(os: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    os.encode_wide().flat_map(|u| u.to_le_bytes()).collect()
}

#[cfg(not(any(unix, windows)))]
fn os_native_bytes(os: &std::ffi::OsStr) -> Vec<u8> {
    os.to_string_lossy().into_owned().into_bytes()
}

fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn collect_files(
    root: &Path,
    current: &Path,
    out: &mut Vec<(String, PathBuf, bool)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        // Check symlink first: a symlink's own file type is neither directory
        // nor file, so it would otherwise be dropped from the tree entirely.
        if file_type.is_symlink() {
            out.push((rel_path(root, &path), path, true));
        } else if file_type.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            collect_files(root, &path, out)?;
        } else if file_type.is_file() {
            out.push((rel_path(root, &path), path, false));
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
    // The version becomes a directory name under the cache, so it must be a
    // single, in-tree path component: reject a separator, `..`, drive colon, or
    // control character that could climb out of the cache root.
    if !crate::pkg::is_safe_cache_component(r) {
        return Err(LockError::InvalidConstraint {
            source: dep_source(&dep.path),
            value: r.to_string(),
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
    fn conflicting_versions_compares_source_case_insensitively() {
        // A GitHub path is case-insensitive, so two casings of one repo at
        // different versions are a single conflict (issue #724).
        let pkg = |source: &str, version: &str| LockedPackage {
            source: source.to_string(),
            version: version.to_string(),
            hash: "sha256:00".to_string(),
        };
        let differ = vec![
            pkg("github.com/Acme/Demo", "v1.0.0"),
            pkg("github.com/acme/demo", "v2.0.0"),
        ];
        assert!(
            conflicting_versions(&differ).is_some(),
            "casing variants at different versions must conflict"
        );
        // The same repo at the same version (different casing) is not a conflict.
        let same = vec![
            pkg("github.com/Acme/Demo", "v1.0.0"),
            pkg("github.com/acme/demo", "v1.0.0"),
        ];
        assert!(conflicting_versions(&same).is_none());
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
    fn validate_rehashes_content_despite_poisoned_sidecar() {
        let cache = TempCache::new("poison");
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
        let locked_hash = lock.packages[0].hash.clone();

        // Tamper a cached file, then poison the sidecar so its metadata
        // signature matches the tampered tree while it still asserts the old,
        // locked hash. The cheap-signature shortcut would accept this, but
        // validation must re-hash the content and reject it.
        let f = dir.join("rv.toml");
        std::fs::write(&f, "[package]\nname = \"bar\"\nversion = \"6.6.6\"\n").unwrap();
        let tampered_sig = tree_signature(&dir).expect("signature");
        let sidecar = pkg::hash_sidecar(&dir);
        std::fs::write(&sidecar, format!("{}\n{}\n", locked_hash, tampered_sig)).unwrap();

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
    fn changed_dep_version_in_lock_forces_fresh_resolve() {
        let cache = TempCache::new("verbump");
        cache.seed(
            "github.com/acme/foo",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n",
            )],
        );

        // Lock pins foo@v1.0.0; the manifest now asks for v2.0.0. The lock
        // must not be treated as covering the new request just because the
        // source path matches (issue #528).
        let v1 = manifest_with(&[("github.com/acme/foo", "v1.0.0")]);
        let lock = resolve_and_lock_in(&v1, &cache.root).expect("resolve v1");
        assert!(lock.covers(&v1), "lock covers the version it pinned");

        let v2 = manifest_with(&[("github.com/acme/foo", "v2.0.0")]);
        assert!(
            !lock.covers(&v2),
            "lock must not cover a changed dependency version"
        );
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
    fn from_toml_drops_exact_duplicate_entries() {
        // A lock that repeats an identical package block parses to a single
        // entry, so a dependency's FFI sources are not compiled twice.
        let text = "version = 1\n\
            \n[[package]]\nsource = \"github.com/acme/dup\"\nversion = \"v1\"\nhash = \"sha256:aa\"\n\
            \n[[package]]\nsource = \"github.com/acme/dup\"\nversion = \"v1\"\nhash = \"sha256:aa\"\n";
        let parsed = LockFile::from_toml_str(text).expect("parse");
        assert_eq!(parsed.packages.len(), 1, "duplicate entry is collapsed");
        assert_eq!(parsed.packages[0].source, "github.com/acme/dup");
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

    #[cfg(unix)]
    #[test]
    fn tree_hash_distinguishes_non_unicode_names() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        // Two distinct, non-Unicode file names with identical content. They
        // share a lossy form, so a lossy hash would collide; the lossless hash
        // must keep them apart.
        let cache = TempCache::new("nonutf8");
        let a = cache.root.join("a");
        let b = cache.root.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join(OsStr::from_bytes(b"x\xff")), "same").unwrap();
        std::fs::write(b.join(OsStr::from_bytes(b"x\xfe")), "same").unwrap();
        assert_ne!(
            tree_hash(&a).unwrap(),
            tree_hash(&b).unwrap(),
            "distinct non-Unicode names must hash differently"
        );
    }

    #[cfg(unix)]
    #[test]
    fn tree_hash_bounds_the_symlink_target() {
        use std::os::unix::fs::symlink;
        // Two trees that differ only in where the symlink-target / next-name
        // boundary falls. Without a length prefix the target bytes run into the
        // next entry's path and both trees hash the same.
        let cache = TempCache::new("symboundary");
        let a = cache.root.join("a");
        let b = cache.root.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        symlink("xy", a.join("s")).unwrap();
        std::fs::write(a.join("z"), "C").unwrap();
        symlink("x", b.join("s")).unwrap();
        std::fs::write(b.join("yz"), "C").unwrap();
        assert_ne!(
            tree_hash(&a).unwrap(),
            tree_hash(&b).unwrap(),
            "a symlink target must not bleed into the next entry"
        );
    }

    #[test]
    fn hash_with_cache_persists_beside_dir_and_reuses() {
        let cache = TempCache::new("hashcache");
        let dir = cache.seed("github.com/acme/bar", "v1.0.0", &[("a.rv", "one")]);
        let sidecar = pkg::hash_sidecar(&dir);
        assert!(!sidecar.exists());

        let first = hash_with_cache(&dir).expect("first");
        assert!(sidecar.exists(), "a sidecar is written");
        // The sidecar lives beside the version dir, so it does not perturb the
        // tree hash of that dir.
        assert_eq!(first, tree_hash(&dir).expect("direct"));
        // A second call returns the same hash from the sidecar.
        assert_eq!(first, hash_with_cache(&dir).expect("second"));
    }

    #[test]
    fn par_map_preserves_order() {
        let input: Vec<usize> = (0..50).collect();
        let out = par_map(input, |x| x * 2);
        let expected: Vec<usize> = (0..50).map(|x| x * 2).collect();
        assert_eq!(out, expected);
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

    #[test]
    fn resolve_rejects_two_versions_of_one_source() {
        // a depends on shared@v1, b depends on shared@v2. Resolution keys
        // packages by source, so the diamond is rejected rather than silently
        // collapsed to one version. Regression for #573.
        let cache = TempCache::new("conflict");
        cache.seed(
            "github.com/acme/a",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"a\"\nversion = \"1.0.0\"\n\n[dependencies]\n\"github.com/acme/shared\" = \"v1.0.0\"\n",
            )],
        );
        cache.seed(
            "github.com/acme/b",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"b\"\nversion = \"1.0.0\"\n\n[dependencies]\n\"github.com/acme/shared\" = \"v2.0.0\"\n",
            )],
        );
        cache.seed(
            "github.com/acme/shared",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"shared\"\nversion = \"1.0.0\"\n",
            )],
        );
        cache.seed(
            "github.com/acme/shared",
            "v2.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"shared\"\nversion = \"2.0.0\"\n",
            )],
        );

        let manifest = manifest_with(&[
            ("github.com/acme/a", "v1.0.0"),
            ("github.com/acme/b", "v1.0.0"),
        ]);
        let err = resolve_and_lock_in(&manifest, &cache.root).unwrap_err();
        match err {
            LockError::ConflictingVersions { source, versions } => {
                assert_eq!(source, "github.com/acme/shared");
                assert_eq!(versions, vec!["v1.0.0".to_string(), "v2.0.0".to_string()]);
            }
            other => panic!("expected ConflictingVersions, got {:?}", other),
        }
    }

    #[test]
    fn transitive_coverage_detects_a_missing_subdependency() {
        // a@v1 declares a dependency on old@v1, but the lock lists only a@v1.
        // The lock is therefore not transitively complete. Regression for #576.
        let cache = TempCache::new("incomplete");
        cache.seed(
            "github.com/acme/a",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"a\"\nversion = \"1.0.0\"\n\n[dependencies]\n\"github.com/acme/old\" = \"v1.0.0\"\n",
            )],
        );
        cache.seed(
            "github.com/acme/old",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"old\"\nversion = \"1.0.0\"\n",
            )],
        );

        // A lock that omits old@v1.
        let partial = LockFile {
            version: LOCK_VERSION,
            packages: vec![LockedPackage {
                source: "github.com/acme/a".to_string(),
                version: "v1.0.0".to_string(),
                hash: tree_hash(&pkg::cache_dir_in(
                    &cache.root,
                    "github.com",
                    "acme",
                    "a",
                    "v1.0.0",
                ))
                .expect("hash"),
            }],
        };
        assert!(
            !lock_covers_transitive(&partial, &cache.root).expect("check"),
            "a lock missing a declared sub-dependency is not transitively complete"
        );

        // The complete lock is covered.
        let manifest = manifest_with(&[("github.com/acme/a", "v1.0.0")]);
        let full = resolve_and_lock_in(&manifest, &cache.root).expect("resolve");
        assert!(
            lock_covers_transitive(&full, &cache.root).expect("check"),
            "a freshly resolved lock is transitively complete"
        );
    }
}
