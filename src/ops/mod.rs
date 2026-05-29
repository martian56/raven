//! High-level rvpm operations: add, install, and update.
//!
//! These wire `raven::manifest`, `raven::pkg`, and `raven::lock` into the
//! workflows the `rvpm` binary exposes. The binary stays thin and calls
//! these functions; the logic lives here so it is unit-testable against an
//! explicit cache root (the `_in` variants) without the global
//! `RVPM_CACHE_DIR` environment variable.
//!
//! Constraint model: constraints in `rv.toml` are literal git refs today
//! (see `raven::lock`). `add` records the requested ref verbatim, and
//! `update` re-resolves the refs as written. Range-based selection is
//! future work.
//!
//! See `docs/v2/specs/rvpm.md` for the command semantics.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::driver::{self, DriverError};
use crate::lock::{self, LockError, LockFile, LOCK_FILE_NAME};
use crate::manifest::{Manifest, ManifestError};
use crate::pkg;
use crate::resolve::{GithubPath, PackageContext};

/// The default constraint written for `rvpm add` when no `@version` is
/// given. Real latest-tag resolution is future work, so the placeholder is
/// recorded verbatim and resolution of it will fail until a concrete ref is
/// supplied. Callers should prefer `add github.com/user/repo@<ref>`.
pub const DEFAULT_ADD_CONSTRAINT: &str = "latest";

/// The conventional manifest file name beside a lock.
pub const MANIFEST_FILE_NAME: &str = "rv.toml";

/// An error produced by a high-level rvpm operation.
#[derive(Debug)]
pub enum OpError {
    /// A filesystem operation failed.
    Io {
        action: String,
        path: PathBuf,
        source: std::io::Error,
    },
    /// The package path argument was not a `github.com/<user>/<repo>` path.
    InvalidPath(String),
    /// Reading or parsing a manifest failed.
    Manifest(ManifestError),
    /// Editing `rv.toml` produced TOML that no longer parses.
    ManifestEdit(String),
    /// Resolving, locking, reading, or validating the lock failed.
    Lock(LockError),
    /// `update` named a package that is not a dependency in `rv.toml`.
    UnknownPackage(String),
    /// The package entry file (`src/main.rv`) was not found.
    MissingEntry(PathBuf),
    /// Compiling or linking the package failed.
    Build(DriverError),
    /// Running the produced binary failed to launch.
    Run {
        binary: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpError::Io {
                action,
                path,
                source,
            } => write!(f, "cannot {} '{}': {}", action, path.display(), source),
            OpError::InvalidPath(p) => {
                write!(f, "'{}' is not a 'github.com/<user>/<repo>' path", p)
            }
            OpError::Manifest(e) => write!(f, "{}", e),
            OpError::ManifestEdit(msg) => write!(f, "could not update rv.toml: {}", msg),
            OpError::Lock(e) => write!(f, "{}", e),
            OpError::UnknownPackage(p) => {
                write!(f, "'{}' is not a dependency in rv.toml", p)
            }
            OpError::MissingEntry(p) => {
                write!(f, "package entry file not found: '{}'", p.display())
            }
            OpError::Build(e) => write!(f, "{}", e),
            OpError::Run { binary, source } => {
                write!(f, "cannot run '{}': {}", binary.display(), source)
            }
        }
    }
}

impl std::error::Error for OpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OpError::Io { source, .. } => Some(source),
            OpError::Manifest(e) => Some(e),
            OpError::Lock(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ManifestError> for OpError {
    fn from(e: ManifestError) -> Self {
        OpError::Manifest(e)
    }
}

impl From<LockError> for OpError {
    fn from(e: LockError) -> Self {
        OpError::Lock(e)
    }
}

impl From<DriverError> for OpError {
    fn from(e: DriverError) -> Self {
        OpError::Build(e)
    }
}

/// What an `add` did to `rv.toml`, for reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddOutcome {
    /// The dependency was new and was appended.
    Added,
    /// The dependency already existed with a different version and was
    /// updated. Carries the previous constraint.
    Updated { previous: String },
    /// The dependency already existed at the same version; nothing changed.
    Unchanged,
}

/// The result of an `add`, install, or update for the binary to print.
#[derive(Debug, Clone)]
pub struct OpReport {
    pub outcome_lines: Vec<String>,
    pub package_count: usize,
}

/// Add a dependency to `rv.toml`, then resolve and write `rv.lock`, using
/// the default cache root. See [`add_in`].
pub fn add(project_dir: &Path, path: &str, version: Option<&str>) -> Result<OpReport, OpError> {
    add_in(project_dir, path, version, &pkg::cache_root())
}

/// Add (or update) a dependency in `rv.toml` under `project_dir`, then
/// re-resolve against `cache_root` and rewrite `rv.lock`.
///
/// `version` is the requested git ref (the part after `@`). When `None`,
/// [`DEFAULT_ADD_CONSTRAINT`] is recorded. An existing entry with a
/// different version is updated, not duplicated, and the change is reported.
pub fn add_in(
    project_dir: &Path,
    path: &str,
    version: Option<&str>,
    cache_root: &Path,
) -> Result<OpReport, OpError> {
    let gh = GithubPath::parse(path).ok_or_else(|| OpError::InvalidPath(path.to_string()))?;
    let key = format!("github.com/{}/{}", gh.user, gh.repo);
    let constraint = version.unwrap_or(DEFAULT_ADD_CONSTRAINT);

    let manifest_path = project_dir.join(MANIFEST_FILE_NAME);
    let text = read_to_string(&manifest_path)?;
    let (new_text, outcome) = upsert_dependency(&text, &key, constraint)?;

    // Guard: the edited text must still parse as a manifest.
    Manifest::from_toml_str(&new_text)?;
    write_file(&manifest_path, &new_text)?;

    let manifest = Manifest::from_toml_str(&new_text)?;
    let lock = lock::resolve_and_lock_in(&manifest, cache_root)?;
    let lock_path = project_dir.join(LOCK_FILE_NAME);
    lock.write(&lock_path)?;

    let mut lines = Vec::new();
    match &outcome {
        AddOutcome::Added => lines.push(format!("Added {} {}", key, constraint)),
        AddOutcome::Updated { previous } => lines.push(format!(
            "Updated {} from {} to {}",
            key, previous, constraint
        )),
        AddOutcome::Unchanged => {
            lines.push(format!("{} {} is already in rv.toml", key, constraint))
        }
    }
    lines.push(format!(
        "Wrote {} with {} package(s)",
        LOCK_FILE_NAME,
        lock.packages.len()
    ));
    Ok(OpReport {
        outcome_lines: lines,
        package_count: lock.packages.len(),
    })
}

/// What an install did, for reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    /// An existing lock covered the manifest and was validated.
    Validated,
    /// No usable lock was present; a fresh one was resolved and written.
    Resolved,
}

/// Re-resolve `rv.toml` against `rv.lock` and fill the cache, using the
/// default cache root. See [`install_in`].
pub fn install(project_dir: &Path) -> Result<(InstallOutcome, OpReport), OpError> {
    install_in(project_dir, &pkg::cache_root())
}

/// Install dependencies for the project under `project_dir` against
/// `cache_root`.
///
/// When `rv.lock` exists and covers every direct dependency, the lock is
/// validated: each pinned entry is fetched and its tree hash is verified. A
/// mismatch aborts. When the lock is missing or does not cover the
/// manifest, a fresh lock is resolved and written.
pub fn install_in(
    project_dir: &Path,
    cache_root: &Path,
) -> Result<(InstallOutcome, OpReport), OpError> {
    let manifest_path = project_dir.join(MANIFEST_FILE_NAME);
    let manifest = Manifest::load(&manifest_path)?;
    let lock_path = project_dir.join(LOCK_FILE_NAME);

    if lock_path.exists() {
        let existing = LockFile::load(&lock_path)?;
        if existing.covers(&manifest) {
            lock::validate_lock_in(&existing, cache_root)?;
            let count = existing.packages.len();
            return Ok((
                InstallOutcome::Validated,
                OpReport {
                    outcome_lines: vec![format!("Validated {} package(s) against rv.lock", count)],
                    package_count: count,
                },
            ));
        }
    }

    let lock = lock::resolve_and_lock_in(&manifest, cache_root)?;
    lock.write(&lock_path)?;
    let count = lock.packages.len();
    Ok((
        InstallOutcome::Resolved,
        OpReport {
            outcome_lines: vec![format!(
                "Resolved {} package(s) and wrote {}",
                count, LOCK_FILE_NAME
            )],
            package_count: count,
        },
    ))
}

/// Re-resolve and rewrite `rv.lock` for one package (or all), using the
/// default cache root. See [`update_in`].
pub fn update(project_dir: &Path, package: Option<&str>) -> Result<OpReport, OpError> {
    update_in(project_dir, package, &pkg::cache_root())
}

/// Update pinned versions in `rv.lock` by re-resolving `rv.toml` against
/// `cache_root`.
///
/// With no `package`, every entry is re-resolved from `rv.toml` and the
/// whole lock is rewritten. With a `package` path, only that dependency is
/// re-resolved; its lock entry (and its transitive entries) are refreshed
/// while the rest of the lock is preserved. Because constraints are literal
/// refs today, an update picks up a ref edited in `rv.toml`. Range-based
/// bumping is future work.
pub fn update_in(
    project_dir: &Path,
    package: Option<&str>,
    cache_root: &Path,
) -> Result<OpReport, OpError> {
    let manifest_path = project_dir.join(MANIFEST_FILE_NAME);
    let manifest = Manifest::load(&manifest_path)?;
    let lock_path = project_dir.join(LOCK_FILE_NAME);

    let full = lock::resolve_and_lock_in(&manifest, cache_root)?;

    match package {
        None => {
            full.write(&lock_path)?;
            Ok(OpReport {
                outcome_lines: vec![format!(
                    "Updated {} with {} package(s)",
                    LOCK_FILE_NAME,
                    full.packages.len()
                )],
                package_count: full.packages.len(),
            })
        }
        Some(path) => {
            let gh =
                GithubPath::parse(path).ok_or_else(|| OpError::InvalidPath(path.to_string()))?;
            let key = format!("github.com/{}/{}", gh.user, gh.repo);
            if !manifest.dependencies.iter().any(|d| d.path == key) {
                return Err(OpError::UnknownPackage(key));
            }

            // Re-resolve only the named dependency's subgraph, then merge its
            // fresh entries over a preserved copy of the existing lock.
            let single = single_dep_manifest(&manifest, &key);
            let refreshed = lock::resolve_and_lock_in(&single, cache_root)?;

            let mut merged = if lock_path.exists() {
                LockFile::load(&lock_path)?
            } else {
                full.clone()
            };
            for entry in &refreshed.packages {
                if let Some(existing) = merged
                    .packages
                    .iter_mut()
                    .find(|p| p.source == entry.source)
                {
                    *existing = entry.clone();
                } else {
                    merged.packages.push(entry.clone());
                }
            }
            merged
                .packages
                .sort_by(|a, b| a.source.cmp(&b.source).then(a.version.cmp(&b.version)));
            merged.write(&lock_path)?;
            Ok(OpReport {
                outcome_lines: vec![format!("Updated {} in {}", key, LOCK_FILE_NAME)],
                package_count: refreshed.packages.len(),
            })
        }
    }
}

/// The conventional package entry source, relative to the project root.
pub const ENTRY_FILE: &str = "src/main.rv";

/// The output directory for a built package binary, relative to the
/// project root.
pub const OUTPUT_DIR: &str = "target/raven-out";

/// The result of a `build`: the path to the produced binary and the lines
/// to report.
#[derive(Debug, Clone)]
pub struct BuildReport {
    pub binary: PathBuf,
    pub outcome_lines: Vec<String>,
}

/// Build the package under `project_dir`, using the default cache root.
/// See [`build_in`].
pub fn build(project_dir: &Path) -> Result<BuildReport, OpError> {
    build_in(project_dir, &pkg::cache_root())
}

/// Build the package under `project_dir` against `cache_root`.
///
/// Ensures dependencies are installed (resolving and writing `rv.lock` if
/// needed, or validating an existing lock against the cache), then loads
/// the lock to build a [`PackageContext`] so external (`github.com/...`)
/// imports resolve through the cache, compiles the entry file
/// (`src/main.rv`), and writes the binary to
/// `target/raven-out/<package-name>` (with `.exe` on Windows).
pub fn build_in(project_dir: &Path, cache_root: &Path) -> Result<BuildReport, OpError> {
    let manifest_path = project_dir.join(MANIFEST_FILE_NAME);
    let manifest = Manifest::load(&manifest_path)?;

    // Ensure the lock exists and the cache is populated. `install_in`
    // validates an up-to-date lock or resolves a fresh one.
    let (_outcome, _report) = install_in(project_dir, cache_root)?;

    let lock_path = project_dir.join(LOCK_FILE_NAME);
    let lock = if lock_path.exists() {
        LockFile::load(&lock_path)?
    } else {
        LockFile {
            version: lock::LOCK_VERSION,
            packages: Vec::new(),
        }
    };
    let ctx = PackageContext::new(cache_root.to_path_buf(), &lock);

    let entry = project_dir.join(ENTRY_FILE);
    if !entry.is_file() {
        return Err(OpError::MissingEntry(entry));
    }

    let binary = output_binary_path(project_dir, &manifest.package.name);
    if let Some(parent) = binary.parent() {
        std::fs::create_dir_all(parent).map_err(|e| OpError::Io {
            action: "create output directory".to_string(),
            path: parent.to_path_buf(),
            source: e,
        })?;
    }

    driver::build_binary(&entry, &binary, Some(&ctx))?;

    Ok(BuildReport {
        binary: binary.clone(),
        outcome_lines: vec![format!(
            "Compiled {} to {}",
            manifest.package.name,
            binary.display()
        )],
    })
}

/// Build the package then run the produced binary, forwarding `args` to it
/// and returning its exit code. Uses the default cache root.
pub fn run_package(project_dir: &Path, args: &[String]) -> Result<i32, OpError> {
    run_package_in(project_dir, args, &pkg::cache_root())
}

/// Build the package under `project_dir` against `cache_root`, then run the
/// produced binary with `args`, returning its exit code.
pub fn run_package_in(
    project_dir: &Path,
    args: &[String],
    cache_root: &Path,
) -> Result<i32, OpError> {
    let report = build_in(project_dir, cache_root)?;
    let status = std::process::Command::new(&report.binary)
        .args(args)
        .status()
        .map_err(|e| OpError::Run {
            binary: report.binary.clone(),
            source: e,
        })?;
    Ok(status.code().unwrap_or(1))
}

/// The output binary path for a package: `target/raven-out/<name>` with
/// the platform executable extension.
fn output_binary_path(project_dir: &Path, name: &str) -> PathBuf {
    let mut p = project_dir.join(OUTPUT_DIR);
    if cfg!(windows) {
        p.push(format!("{}.exe", name));
    } else {
        p.push(name);
    }
    p
}

/// Build a manifest carrying only the one dependency `key`, reusing the
/// package identity from `base`.
fn single_dep_manifest(base: &Manifest, key: &str) -> Manifest {
    let mut m = base.clone();
    m.dependencies.retain(|d| d.path == key);
    m
}

/// Insert or update a `[dependencies]` entry in `toml_text`, preserving
/// formatting and comments. Returns the new text and what changed.
fn upsert_dependency(
    toml_text: &str,
    key: &str,
    constraint: &str,
) -> Result<(String, AddOutcome), OpError> {
    use toml_edit::{value, DocumentMut, Item, Table};

    let mut doc: DocumentMut = toml_text
        .parse()
        .map_err(|e: toml_edit::TomlError| OpError::ManifestEdit(e.to_string()))?;

    if doc.get("dependencies").is_none() {
        let mut t = Table::new();
        t.set_implicit(false);
        doc["dependencies"] = Item::Table(t);
    }
    let deps = doc["dependencies"]
        .as_table_mut()
        .ok_or_else(|| OpError::ManifestEdit("[dependencies] is not a table".to_string()))?;

    let outcome = match deps.get(key).and_then(|i| i.as_str()) {
        Some(prev) if prev == constraint => AddOutcome::Unchanged,
        Some(prev) => AddOutcome::Updated {
            previous: prev.to_string(),
        },
        None => AddOutcome::Added,
    };
    deps[key] = value(constraint);

    Ok((doc.to_string(), outcome))
}

fn read_to_string(path: &Path) -> Result<String, OpError> {
    std::fs::read_to_string(path).map_err(|e| OpError::Io {
        action: "read".to_string(),
        path: path.to_path_buf(),
        source: e,
    })
}

fn write_file(path: &Path, contents: &str) -> Result<(), OpError> {
    std::fs::write(path, contents).map_err(|e| OpError::Io {
        action: "write".to_string(),
        path: path.to_path_buf(),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempProject {
        root: PathBuf,
    }

    impl TempProject {
        fn new(tag: &str) -> TempProject {
            let mut root = std::env::temp_dir();
            root.push(format!(
                "rvpm-ops-{}-{}-{}",
                tag,
                std::process::id(),
                next_counter()
            ));
            std::fs::create_dir_all(&root).expect("create temp project");
            TempProject { root }
        }

        fn write_manifest(&self, text: &str) {
            std::fs::write(self.root.join(MANIFEST_FILE_NAME), text).expect("write rv.toml");
        }

        fn manifest_text(&self) -> String {
            std::fs::read_to_string(self.root.join(MANIFEST_FILE_NAME)).expect("read rv.toml")
        }

        fn lock(&self) -> LockFile {
            LockFile::load(self.root.join(LOCK_FILE_NAME)).expect("load rv.lock")
        }

        /// Seed a cache entry under `cache_root` for a package version.
        fn seed(&self, cache_root: &Path, source: &str, version: &str, files: &[(&str, &str)]) {
            let gh = GithubPath::parse(source).expect("github path");
            let dir = pkg::cache_dir_in(cache_root, &gh.host, &gh.user, &gh.repo, version);
            std::fs::create_dir_all(&dir).expect("create cache dir");
            for (rel, contents) in files {
                let path = dir.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("create parent");
                }
                std::fs::write(&path, contents).expect("write seed");
            }
        }

        fn cache_root(&self) -> PathBuf {
            self.root.join("cache")
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn next_counter() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        COUNTER.fetch_add(1, Ordering::Relaxed)
    }

    const APP_MANIFEST: &str =
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n# keep this comment\n";

    #[test]
    fn add_appends_dependency_and_writes_lock() {
        let proj = TempProject::new("add");
        proj.write_manifest(APP_MANIFEST);
        let cache = proj.cache_root();
        proj.seed(
            &cache,
            "github.com/acme/foo",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n",
            )],
        );

        let report =
            add_in(&proj.root, "github.com/acme/foo", Some("v1.0.0"), &cache).expect("add");
        assert_eq!(report.package_count, 1);

        let text = proj.manifest_text();
        assert!(
            text.contains("github.com/acme/foo"),
            "dep written: {}",
            text
        );
        assert!(text.contains("keep this comment"), "comment preserved");
        let m = Manifest::from_toml_str(&text).expect("re-parses");
        assert!(m
            .dependencies
            .iter()
            .any(|d| d.path == "github.com/acme/foo"));

        let lock = proj.lock();
        let foo = lock
            .packages
            .iter()
            .find(|p| p.source == "github.com/acme/foo")
            .expect("foo in lock");
        assert_eq!(foo.version, "v1.0.0");
        assert!(foo.hash.starts_with("sha256:"));
    }

    #[test]
    fn add_updates_existing_different_version() {
        let proj = TempProject::new("addupd");
        proj.write_manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/foo\" = \"v1.0.0\"\n",
        );
        let cache = proj.cache_root();
        proj.seed(
            &cache,
            "github.com/acme/foo",
            "v1.1.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.1.0\"\n",
            )],
        );

        let report =
            add_in(&proj.root, "github.com/acme/foo", Some("v1.1.0"), &cache).expect("add");
        assert!(report
            .outcome_lines
            .iter()
            .any(|l| l.contains("Updated") && l.contains("v1.0.0") && l.contains("v1.1.0")));

        let m = Manifest::from_toml_str(&proj.manifest_text()).expect("re-parses");
        let foo = m
            .dependencies
            .iter()
            .find(|d| d.path == "github.com/acme/foo")
            .unwrap();
        assert_eq!(foo.constraint, "v1.1.0");
        assert_eq!(proj.lock().packages[0].version, "v1.1.0");
    }

    #[test]
    fn install_validates_existing_lock() {
        let proj = TempProject::new("installok");
        proj.write_manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/foo\" = \"v1.0.0\"\n",
        );
        let cache = proj.cache_root();
        proj.seed(
            &cache,
            "github.com/acme/foo",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n",
            )],
        );
        // First install resolves a fresh lock.
        let (outcome, _) = install_in(&proj.root, &cache).expect("first install");
        assert_eq!(outcome, InstallOutcome::Resolved);
        // Second install validates it.
        let (outcome, _) = install_in(&proj.root, &cache).expect("second install");
        assert_eq!(outcome, InstallOutcome::Validated);
    }

    #[test]
    fn install_detects_tampered_cache() {
        let proj = TempProject::new("tamper");
        proj.write_manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/foo\" = \"v1.0.0\"\n",
        );
        let cache = proj.cache_root();
        proj.seed(
            &cache,
            "github.com/acme/foo",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n",
            )],
        );
        let (outcome, _) = install_in(&proj.root, &cache).expect("resolve lock");
        assert_eq!(outcome, InstallOutcome::Resolved);

        // Tamper with a cached file after locking.
        let gh = GithubPath::parse("github.com/acme/foo").unwrap();
        let dir = pkg::cache_dir_in(&cache, &gh.host, &gh.user, &gh.repo, "v1.0.0");
        let f = dir.join("rv.toml");
        let mut contents = std::fs::read_to_string(&f).unwrap();
        contents.push_str("\n# tampered\n");
        std::fs::write(&f, contents).unwrap();

        let err = install_in(&proj.root, &cache).unwrap_err();
        match err {
            OpError::Lock(LockError::HashMismatch { source, .. }) => {
                assert_eq!(source, "github.com/acme/foo");
            }
            other => panic!("expected HashMismatch, got {:?}", other),
        }
    }

    #[test]
    fn install_without_lock_writes_fresh() {
        let proj = TempProject::new("nolock");
        proj.write_manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/foo\" = \"v1.0.0\"\n",
        );
        let cache = proj.cache_root();
        proj.seed(
            &cache,
            "github.com/acme/foo",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n",
            )],
        );
        assert!(!proj.root.join(LOCK_FILE_NAME).exists());
        let (outcome, report) = install_in(&proj.root, &cache).expect("install");
        assert_eq!(outcome, InstallOutcome::Resolved);
        assert_eq!(report.package_count, 1);
        assert!(proj.root.join(LOCK_FILE_NAME).exists());
    }

    #[test]
    fn update_bumps_locked_version() {
        let proj = TempProject::new("update");
        // Start pinned at v1.0.0.
        proj.write_manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/foo\" = \"v1.0.0\"\n",
        );
        let cache = proj.cache_root();
        proj.seed(
            &cache,
            "github.com/acme/foo",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n",
            )],
        );
        proj.seed(
            &cache,
            "github.com/acme/foo",
            "v1.1.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.1.0\"\n",
            )],
        );
        let (_, _) = install_in(&proj.root, &cache).expect("install");
        let before = proj.lock();
        assert_eq!(before.packages[0].version, "v1.0.0");
        let old_hash = before.packages[0].hash.clone();

        // Bump the ref in rv.toml, then update just that package.
        proj.write_manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/foo\" = \"v1.1.0\"\n",
        );
        update_in(&proj.root, Some("github.com/acme/foo"), &cache).expect("update");

        let after = proj.lock();
        let foo = after
            .packages
            .iter()
            .find(|p| p.source == "github.com/acme/foo")
            .unwrap();
        assert_eq!(foo.version, "v1.1.0");
        assert_ne!(foo.hash, old_hash);
    }

    #[test]
    fn update_unknown_package_is_rejected() {
        let proj = TempProject::new("updunknown");
        proj.write_manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/foo\" = \"v1.0.0\"\n",
        );
        let cache = proj.cache_root();
        proj.seed(
            &cache,
            "github.com/acme/foo",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n",
            )],
        );
        let err = update_in(&proj.root, Some("github.com/acme/bar"), &cache).unwrap_err();
        assert!(matches!(err, OpError::UnknownPackage(_)));
    }

    #[test]
    fn build_reports_missing_entry() {
        // A project with no dependencies and no src/main.rv: install
        // resolves an empty lock, then build fails on the missing entry
        // before any compilation or linking is attempted.
        let proj = TempProject::new("noentry");
        proj.write_manifest("[package]\nname = \"app\"\nversion = \"0.1.0\"\n");
        let cache = proj.cache_root();
        std::fs::create_dir_all(&cache).expect("mkdir cache");
        let err = build_in(&proj.root, &cache).unwrap_err();
        match err {
            OpError::MissingEntry(p) => {
                assert!(p.ends_with("src/main.rv") || p.ends_with("src\\main.rv"))
            }
            other => panic!("expected MissingEntry, got {:?}", other),
        }
    }

    #[test]
    fn output_binary_path_uses_package_name() {
        let p = output_binary_path(Path::new("/proj"), "demo");
        if cfg!(windows) {
            assert!(
                p.ends_with("target/raven-out/demo.exe")
                    || p.ends_with("target\\raven-out\\demo.exe")
            );
        } else {
            assert!(p.ends_with("target/raven-out/demo"));
        }
    }

    #[test]
    fn upsert_creates_dependencies_section() {
        let (text, outcome) =
            upsert_dependency(APP_MANIFEST, "github.com/acme/foo", "v1.0.0").expect("upsert");
        assert_eq!(outcome, AddOutcome::Added);
        assert!(text.contains("[dependencies]"));
        assert!(Manifest::from_toml_str(&text).is_ok());
    }

    #[test]
    fn upsert_same_version_is_unchanged() {
        let base = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/foo\" = \"v1.0.0\"\n";
        let (_, outcome) =
            upsert_dependency(base, "github.com/acme/foo", "v1.0.0").expect("upsert");
        assert_eq!(outcome, AddOutcome::Unchanged);
    }
}
