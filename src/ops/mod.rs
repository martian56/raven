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

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::codegen::linker;
use crate::driver::{self, DriverError};
use crate::lock::{self, LockError, LockFile, LOCK_FILE_NAME};
use crate::manifest::{Ffi, Manifest, ManifestError};
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
    /// Neither entry file (`src/main.rv` for an application nor `lib.rv` for a
    /// library) was found.
    MissingEntry(PathBuf),
    /// `run` was asked to execute a library, which produces no binary.
    NotRunnable,
    /// A `*_test.rv` file could not be lexed or parsed during test discovery.
    TestParse { file: PathBuf, message: String },
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
                let root = p.parent().and_then(|s| s.parent());
                match root {
                    Some(root) => write!(
                        f,
                        "no package entry in '{}': expected 'src/main.rv' (application) or 'lib.rv' (library)",
                        root.display()
                    ),
                    None => write!(
                        f,
                        "no package entry: expected 'src/main.rv' (application) or 'lib.rv' (library)"
                    ),
                }
            }
            OpError::NotRunnable => write!(
                f,
                "this package is a library (lib.rv) with no executable to run; use 'rvpm build' to type-check it"
            ),
            OpError::TestParse { file, message } => {
                write!(f, "cannot read test file '{}': {}", file.display(), message)
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

/// Collect the native link inputs from a project and its dependencies' `[ffi]`
/// sections, compiling any bundled C sources into a single static archive.
fn gather_native_link(
    project_dir: &Path,
    manifest: &Manifest,
    lock: &LockFile,
    cache_root: &Path,
    out_dir: &Path,
) -> Result<linker::NativeLink, OpError> {
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut libs: Vec<String> = Vec::new();
    let mut link_args: Vec<String> = Vec::new();

    // The project's own [ffi], resolved against the project root.
    collect_ffi(
        &manifest.ffi,
        project_dir,
        &mut sources,
        &mut libs,
        &mut link_args,
    );

    // Each dependency's [ffi], resolved against its cache directory, so a
    // package can bundle its own C and a consumer needs nothing installed.
    for entry in &lock.packages {
        let Some(gh) = GithubPath::parse(&entry.source) else {
            continue;
        };
        let dir = pkg::cache_dir_in(cache_root, &gh.host, &gh.user, &gh.repo, &entry.version);
        let dep_manifest = dir.join(MANIFEST_FILE_NAME);
        if dep_manifest.exists() {
            if let Ok(dep) = Manifest::load(&dep_manifest) {
                collect_ffi(&dep.ffi, &dir, &mut sources, &mut libs, &mut link_args);
            }
        }
    }

    let objects = if sources.is_empty() {
        Vec::new()
    } else {
        linker::compile_c_sources(&sources, out_dir).map_err(DriverError::from)?
    };
    Ok(linker::NativeLink {
        objects,
        libs,
        link_args,
    })
}

fn collect_ffi(
    ffi: &Ffi,
    base: &Path,
    sources: &mut Vec<PathBuf>,
    libs: &mut Vec<String>,
    link_args: &mut Vec<String>,
) {
    for source in &ffi.sources {
        sources.push(base.join(source));
    }
    libs.extend(ffi.libs.iter().cloned());
    link_args.extend(ffi.link_args.iter().cloned());
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
        // The lock is trusted only when it covers the direct dependencies AND
        // contains the complete transitive graph. A lock that lists a package
        // but omits the packages it pulls in is incomplete: fall through and
        // re-resolve a fresh, complete lock instead of building against it.
        if existing.covers(&manifest) {
            lock::validate_lock_in(&existing, cache_root)?;
            if lock::lock_covers_transitive(&existing, cache_root)? {
                let count = existing.packages.len();
                return Ok((
                    InstallOutcome::Validated,
                    OpReport {
                        outcome_lines: vec![format!(
                            "Validated {} package(s) against rv.lock",
                            count
                        )],
                        package_count: count,
                    },
                ));
            }
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
            // Prune packages no longer reachable from the manifest graph. After
            // bumping the named dependency, a package that was only pulled in by
            // its previous version is dead weight; walking the graph from the
            // manifest through the merged pins drops it.
            prune_unreachable(&mut merged, &manifest, cache_root)?;
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

/// Drop every locked package not reachable from `manifest`'s dependency graph.
/// Walks from the manifest's direct dependencies, following each package's
/// declared sub-dependencies (read from its cached `rv.toml`) using the lock's
/// own pins as the version for each source, and retains only the packages
/// visited.
fn prune_unreachable(
    lock: &mut LockFile,
    manifest: &Manifest,
    cache_root: &Path,
) -> Result<(), OpError> {
    let by_source: BTreeMap<String, String> = lock
        .packages
        .iter()
        .map(|p| (p.source.clone(), p.version.clone()))
        .collect();
    let mut reachable: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = manifest
        .dependencies
        .iter()
        .map(|d| d.path.clone())
        .collect();
    while let Some(source) = stack.pop() {
        if !reachable.insert(source.clone()) {
            continue;
        }
        if let Some(version) = by_source.get(&source) {
            for (sub_source, _sub_version) in lock::cached_subdeps(cache_root, &source, version)? {
                stack.push(sub_source);
            }
        }
    }
    lock.packages.retain(|p| reachable.contains(&p.source));
    Ok(())
}

/// The conventional application entry source, relative to the project root.
pub const ENTRY_FILE: &str = "src/main.rv";

/// The conventional library entry source, at the project root. This is the file
/// other projects load for `import "github.com/<user>/<repo>"`.
pub const LIB_ENTRY_FILE: &str = "lib.rv";

/// The output directory for a built package binary, relative to the
/// project root.
pub const OUTPUT_DIR: &str = "target/raven-out";

/// The result of a `build`: the produced binary (`None` for a library, which is
/// type-checked rather than compiled to a binary) and the lines to report.
#[derive(Debug, Clone)]
pub struct BuildReport {
    pub binary: Option<PathBuf>,
    pub outcome_lines: Vec<String>,
}

/// A package's entry: an application compiled to a binary, or a library that is
/// type-checked.
enum Entry {
    App(PathBuf),
    Lib(PathBuf),
}

/// Find the package entry. `src/main.rv` (an application) takes precedence over
/// `lib.rv` (a library) at the project root.
fn resolve_entry(project_dir: &Path) -> Option<Entry> {
    let app = project_dir.join(ENTRY_FILE);
    if app.is_file() {
        return Some(Entry::App(app));
    }
    let lib = project_dir.join(LIB_ENTRY_FILE);
    if lib.is_file() {
        return Some(Entry::Lib(lib));
    }
    None
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

    match resolve_entry(project_dir) {
        Some(Entry::App(entry)) => {
            let binary = output_binary_path(project_dir, &manifest.package.name);
            if let Some(parent) = binary.parent() {
                std::fs::create_dir_all(parent).map_err(|e| OpError::Io {
                    action: "create output directory".to_string(),
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
            // Gather native code to link from the `[ffi]` of the project and
            // its dependencies (bundled C sources, libraries, linker args).
            let ffi_dir = binary
                .parent()
                .map(|p| p.join("ffi"))
                .unwrap_or_else(|| PathBuf::from("ffi"));
            let native = gather_native_link(project_dir, &manifest, &lock, cache_root, &ffi_dir)?;
            driver::build_binary_native(&entry, &binary, Some(&ctx), &native)?;
            Ok(BuildReport {
                outcome_lines: vec![format!(
                    "Compiled {} to {}",
                    manifest.package.name,
                    binary.display()
                )],
                binary: Some(binary),
            })
        }
        Some(Entry::Lib(entry)) => {
            // A library has no `main`, so it is type-checked, not linked.
            let source = std::fs::read_to_string(&entry).map_err(|e| OpError::Io {
                action: "read".to_string(),
                path: entry.clone(),
                source: e,
            })?;
            driver::check(&source, &entry, Some(&ctx))?;
            Ok(BuildReport {
                binary: None,
                outcome_lines: vec![format!(
                    "Checked library {} ({})",
                    manifest.package.name, LIB_ENTRY_FILE
                )],
            })
        }
        None => Err(OpError::MissingEntry(project_dir.join(ENTRY_FILE))),
    }
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
    // A library has no executable; report that before building.
    if let Some(Entry::Lib(_)) = resolve_entry(project_dir) {
        return Err(OpError::NotRunnable);
    }
    let report = build_in(project_dir, cache_root)?;
    let binary = report.binary.ok_or(OpError::NotRunnable)?;
    let status = std::process::Command::new(&binary)
        .args(args)
        .status()
        .map_err(|e| OpError::Run {
            binary: binary.clone(),
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

/// The name of the compiled test-runner binary under `target/raven-out`.
const TEST_BINARY_NAME: &str = ".rvpm-test";

/// The result of a `test` run.
#[derive(Debug, Clone)]
pub struct TestReport {
    pub passed: usize,
    pub failed: usize,
    pub outcome_lines: Vec<String>,
}

/// Discover, compile, and run the package's tests under the default cache.
pub fn test(project_dir: &Path) -> Result<TestReport, OpError> {
    test_in(project_dir, &pkg::cache_root())
}

/// Run every `fun test_*()` found in the package's `*_test.rv` files. Each test
/// runs in its own process (a small generated dispatcher selects the test by
/// name) so a panic from a failed assertion fails only that test, not the run.
pub fn test_in(project_dir: &Path, cache_root: &Path) -> Result<TestReport, OpError> {
    let manifest = Manifest::load(project_dir.join(MANIFEST_FILE_NAME))?;
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

    // Each suite is (relative import path of the test file, test function names).
    let mut suites: Vec<(String, Vec<String>)> = Vec::new();
    for file in discover_test_files(project_dir)? {
        let names = test_functions_in(&file)?;
        if !names.is_empty() {
            suites.push((test_import_path(project_dir, &file), names));
        }
    }

    let total: usize = suites.iter().map(|s| s.1.len()).sum();
    let mut lines = vec![format!(
        "running {} test{}",
        total,
        if total == 1 { "" } else { "s" }
    )];
    if total == 0 {
        lines.push("no tests found (looked for `fun test_*` in `*_test.rv` files)".to_string());
        return Ok(TestReport {
            passed: 0,
            failed: 0,
            outcome_lines: lines,
        });
    }

    // A per-process unique name in the project root (where relative imports
    // resolve), so a test run never overwrites or deletes a user file that
    // happens to be named `.rvpm-test-main.rv`; only the file we create here is
    // removed below.
    let main_path = project_dir.join(format!(".rvpm-test-main-{}.rv", std::process::id()));
    let binary = output_binary_path(project_dir, TEST_BINARY_NAME);
    if let Some(parent) = binary.parent() {
        std::fs::create_dir_all(parent).map_err(|e| OpError::Io {
            action: "create output directory".to_string(),
            path: parent.to_path_buf(),
            source: e,
        })?;
    }

    // Native code from the package's (and its dependencies') `[ffi]`, so a
    // package can test its own FFI bindings. Compiled once and reused per suite.
    let ffi_dir = binary
        .parent()
        .map(|p| p.join("ffi"))
        .unwrap_or_else(|| PathBuf::from("ffi"));
    let native = gather_native_link(project_dir, &manifest, &lock, cache_root, &ffi_dir)?;

    let mut passed = 0usize;
    let mut failed = 0usize;
    // Compile one dispatcher per file, then run each of its tests in isolation.
    // The closure lets the generated entry be removed whether the run succeeds
    // or fails.
    let run = (|| -> Result<(), OpError> {
        for (import, names) in &suites {
            std::fs::write(&main_path, generate_test_dispatcher(import, names)).map_err(|e| {
                OpError::Io {
                    action: "write".to_string(),
                    path: main_path.clone(),
                    source: e,
                }
            })?;
            driver::build_binary_native(&main_path, &binary, Some(&ctx), &native)?;
            for name in names {
                let output = std::process::Command::new(&binary)
                    .arg(name)
                    .output()
                    .map_err(|e| OpError::Run {
                        binary: binary.clone(),
                        source: e,
                    })?;
                if output.status.success() {
                    passed += 1;
                    lines.push(format!("  ok   {}", name));
                } else {
                    failed += 1;
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let reason = stderr.lines().next().unwrap_or("").trim();
                    if reason.is_empty() {
                        lines.push(format!("  FAIL {}", name));
                    } else {
                        lines.push(format!("  FAIL {} ({})", name, reason));
                    }
                }
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&main_path);
    run?;

    lines.push(format!(
        "test result: {}. {} passed; {} failed",
        if failed == 0 { "ok" } else { "FAILED" },
        passed,
        failed
    ));
    Ok(TestReport {
        passed,
        failed,
        outcome_lines: lines,
    })
}

/// Collect every `*_test.rv` file under `project_dir`, skipping the build
/// output and hidden or VCS directories.
fn discover_test_files(project_dir: &Path) -> Result<Vec<PathBuf>, OpError> {
    let mut out = Vec::new();
    collect_test_files(project_dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_test_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), OpError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry.map_err(|e| OpError::Io {
            action: "read directory".to_string(),
            path: dir.to_path_buf(),
            source: e,
        })?;
        let ftype = entry.file_type().map_err(|e| OpError::Io {
            action: "read directory entry".to_string(),
            path: dir.to_path_buf(),
            source: e,
        })?;
        // Do not follow symlinks: a directory link could point outside the
        // package, and a test file reached through it would compile and run
        // code from outside the project tree.
        if ftype.is_symlink() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if ftype.is_dir() {
            if name == "target" || name.starts_with('.') {
                continue;
            }
            collect_test_files(&path, out)?;
        } else if name.ends_with("_test.rv") {
            out.push(path);
        }
    }
    Ok(())
}

/// Parse `file` and return the names of its zero-argument `test_*` functions.
fn test_functions_in(file: &Path) -> Result<Vec<String>, OpError> {
    let source = std::fs::read_to_string(file).map_err(|e| OpError::Io {
        action: "read".to_string(),
        path: file.to_path_buf(),
        source: e,
    })?;
    let tokens = crate::lexer::Lexer::new(source, file.to_path_buf())
        .tokenize()
        .map_err(|e| OpError::TestParse {
            file: file.to_path_buf(),
            message: format!("{}", e),
        })?;
    let parsed = crate::parser::parse(&tokens).map_err(|e| OpError::TestParse {
        file: file.to_path_buf(),
        message: format!("{}", e),
    })?;
    let mut names = Vec::new();
    for item in &parsed.items {
        if let crate::ast::DeclKind::Function(func) = &item.kind {
            if func.name.starts_with("test_") && func.params.is_empty() {
                names.push(func.name.clone());
            }
        }
    }
    Ok(names)
}

/// The `import` path the generated dispatcher uses to reach `file` from the
/// project root: a `./`-prefixed, forward-slashed path with the `.rv`
/// extension dropped.
fn test_import_path(project_dir: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(project_dir).unwrap_or(file);
    let rel = rel.with_extension("");
    let joined = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    format!("./{}", joined)
}

/// Render a runner that imports a test file's functions and calls the one named
/// by the first process argument. Running it per test name isolates panics.
fn generate_test_dispatcher(import: &str, names: &[String]) -> String {
    let mut src = String::from("import std/env { arg_at }\n");
    src.push_str(&format!(
        "import \"{}\" {{ {} }}\n\n",
        import,
        names.join(", ")
    ));
    src.push_str("fun main() {\n    let name = arg_at(1)\n");
    for (i, name) in names.iter().enumerate() {
        let kw = if i == 0 { "if" } else { "} else if" };
        src.push_str(&format!(
            "    {} name.equals(\"{}\") {{\n        {}()\n",
            kw, name, name
        ));
    }
    src.push_str("    }\n}\n");
    src
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

    #[cfg(unix)]
    #[test]
    fn discover_test_files_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;
        let proj = TempProject::new("symlink");
        std::fs::create_dir_all(proj.root.join("src")).expect("create src");
        std::fs::write(proj.root.join("src/inside_test.rv"), "fun test_a() {}\n")
            .expect("write inside test");
        // A directory outside the package, with its own test file.
        let outside = TempProject::new("symlink-outside");
        std::fs::write(outside.root.join("outside_test.rv"), "fun test_b() {}\n")
            .expect("write outside test");
        // A directory symlink inside the package pointing at the outside tree.
        symlink(&outside.root, proj.root.join("linked")).expect("create symlink");

        let found = discover_test_files(&proj.root).expect("discover test files");
        assert!(
            found.iter().any(|p| p.ends_with("inside_test.rv")),
            "the package's own test file must be found: {found:?}"
        );
        assert!(
            !found
                .iter()
                .any(|p| p.to_string_lossy().contains("outside_test")),
            "a test file reached only through a directory symlink must not be discovered: {found:?}"
        );
    }

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
    fn selective_update_prunes_a_removed_transitive_dependency() {
        // a@v1 depends on old@v1; a@v2 has no dependencies. After updating a to
        // v2, old must be pruned from the lock. Regression for #575.
        let proj = TempProject::new("prune");
        proj.write_manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/a\" = \"v1.0.0\"\n",
        );
        let cache = proj.cache_root();
        proj.seed(
            &cache,
            "github.com/acme/a",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"a\"\nversion = \"1.0.0\"\n\n[dependencies]\n\"github.com/acme/old\" = \"v1.0.0\"\n",
            )],
        );
        proj.seed(
            &cache,
            "github.com/acme/old",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"old\"\nversion = \"1.0.0\"\n",
            )],
        );
        proj.seed(
            &cache,
            "github.com/acme/a",
            "v2.0.0",
            &[("rv.toml", "[package]\nname = \"a\"\nversion = \"2.0.0\"\n")],
        );

        install_in(&proj.root, &cache).expect("install");
        let before = proj.lock();
        assert!(
            before
                .packages
                .iter()
                .any(|p| p.source == "github.com/acme/old"),
            "old should be present before the update"
        );

        // Bump a to v2 (which drops old) and update just a.
        proj.write_manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/a\" = \"v2.0.0\"\n",
        );
        update_in(&proj.root, Some("github.com/acme/a"), &cache).expect("update");

        let after = proj.lock();
        assert!(
            after
                .packages
                .iter()
                .any(|p| p.source == "github.com/acme/a" && p.version == "v2.0.0"),
            "a should be bumped to v2"
        );
        assert!(
            !after
                .packages
                .iter()
                .any(|p| p.source == "github.com/acme/old"),
            "old should be pruned, lock was: {:?}",
            after.packages
        );
    }

    #[test]
    fn install_regenerates_a_transitively_incomplete_lock() {
        // The lock lists a@v1 but omits the old@v1 that a@v1 declares. install
        // must detect the gap and re-resolve a complete lock. Regression for
        // #576.
        let proj = TempProject::new("incomplete");
        proj.write_manifest(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/a\" = \"v1.0.0\"\n",
        );
        let cache = proj.cache_root();
        proj.seed(
            &cache,
            "github.com/acme/a",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"a\"\nversion = \"1.0.0\"\n\n[dependencies]\n\"github.com/acme/old\" = \"v1.0.0\"\n",
            )],
        );
        proj.seed(
            &cache,
            "github.com/acme/old",
            "v1.0.0",
            &[(
                "rv.toml",
                "[package]\nname = \"old\"\nversion = \"1.0.0\"\n",
            )],
        );

        // Hand-write a lock that covers the direct dep but omits the transitive
        // old@v1.
        let a_dir = pkg::cache_dir_in(&cache, "github.com", "acme", "a", "v1.0.0");
        let a_hash = crate::lock::tree_hash(&a_dir).expect("hash");
        let partial = format!(
            "version = 1\n\n[[package]]\nsource = \"github.com/acme/a\"\nversion = \"v1.0.0\"\nhash = \"{}\"\n",
            a_hash
        );
        write_file(&proj.root.join(LOCK_FILE_NAME), &partial).expect("write partial lock");

        let (outcome, _) = install_in(&proj.root, &cache).expect("install");
        assert_eq!(
            outcome,
            InstallOutcome::Resolved,
            "an incomplete lock must be regenerated, not validated"
        );
        let after = proj.lock();
        assert!(
            after
                .packages
                .iter()
                .any(|p| p.source == "github.com/acme/old"),
            "the regenerated lock must include the missing transitive dependency"
        );
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
    fn build_type_checks_a_library() {
        // A package with a `lib.rv` and no `src/main.rv` is type-checked rather
        // than compiled to a binary, so the report carries no binary path.
        let proj = TempProject::new("lib");
        proj.write_manifest("[package]\nname = \"mylib\"\nversion = \"0.1.0\"\n");
        std::fs::write(
            proj.root.join(LIB_ENTRY_FILE),
            "fun add(a: Int, b: Int) -> Int {\n    return a + b\n}\n",
        )
        .expect("write lib.rv");
        let cache = proj.cache_root();
        std::fs::create_dir_all(&cache).expect("mkdir cache");
        let report = build_in(&proj.root, &cache).expect("library type-checks");
        assert!(report.binary.is_none(), "a library produces no binary");
        assert!(report
            .outcome_lines
            .iter()
            .any(|l| l.contains("Checked library")));
    }

    #[test]
    fn run_rejects_a_library() {
        // `run` has nothing to execute for a library.
        let proj = TempProject::new("librun");
        proj.write_manifest("[package]\nname = \"mylib\"\nversion = \"0.1.0\"\n");
        std::fs::write(
            proj.root.join(LIB_ENTRY_FILE),
            "fun add(a: Int, b: Int) -> Int {\n    return a + b\n}\n",
        )
        .expect("write lib.rv");
        let cache = proj.cache_root();
        std::fs::create_dir_all(&cache).expect("mkdir cache");
        let err = run_package_in(&proj.root, &[], &cache).unwrap_err();
        assert!(matches!(err, OpError::NotRunnable), "got {:?}", err);
    }

    #[test]
    fn discovers_test_files_and_skips_target() {
        let proj = TempProject::new("disc");
        std::fs::create_dir_all(proj.root.join("src")).unwrap();
        std::fs::create_dir_all(proj.root.join("target")).unwrap();
        std::fs::write(proj.root.join("src").join("math_test.rv"), "").unwrap();
        std::fs::write(proj.root.join("src").join("helper.rv"), "").unwrap();
        std::fs::write(proj.root.join("target").join("stale_test.rv"), "").unwrap();
        let found = discover_test_files(&proj.root).unwrap();
        assert_eq!(found.len(), 1, "found: {:?}", found);
        assert!(found[0].ends_with("math_test.rv"));
    }

    #[test]
    fn collects_zero_arg_test_functions() {
        let proj = TempProject::new("fns");
        let f = proj.root.join("a_test.rv");
        std::fs::write(
            &f,
            "fun test_a() {}\nfun test_b(x: Int) {}\nfun helper() {}\nfun test_c() {}\n",
        )
        .unwrap();
        let names = test_functions_in(&f).unwrap();
        assert_eq!(names, vec!["test_a".to_string(), "test_c".to_string()]);
    }

    #[test]
    fn dispatcher_imports_and_calls_by_name() {
        let src = generate_test_dispatcher(
            "./src/a_test",
            &["test_a".to_string(), "test_b".to_string()],
        );
        assert!(src.contains("import \"./src/a_test\" { test_a, test_b }"));
        assert!(src.contains("if name.equals(\"test_a\") {"));
        assert!(src.contains("} else if name.equals(\"test_b\") {"));
        assert!(src.contains("test_a()"));
        assert!(src.contains("test_b()"));
    }

    #[test]
    fn import_path_is_relative_and_forward_slashed() {
        let proj = TempProject::new("imp");
        let f = proj.root.join("src").join("math_test.rv");
        assert_eq!(test_import_path(&proj.root, &f), "./src/math_test");
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
