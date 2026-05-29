//! The rvpm package manager command line entry point.
//!
//! rvpm manages Raven packages described by an `rv.toml` manifest. This
//! binary is intentionally thin: it parses arguments and dispatches to
//! library code in `raven::manifest`, `raven::pkg`, `raven::lock`, and
//! `raven::ops`.

use std::path::PathBuf;
use std::process::ExitCode;

use raven::lock::{self, LockFile, LOCK_FILE_NAME};
use raven::manifest::init::{init_project, name_from_dir, InitError};
use raven::manifest::Manifest;
use raven::ops;
use raven::pkg;
use raven::resolve::GithubPath;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("help") | Some("--help") | Some("-h") => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some("init") => match cmd_init(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("rvpm: {}", e);
                ExitCode::from(1)
            }
        },
        Some("fetch") => match cmd_fetch(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("rvpm: {}", e);
                ExitCode::from(1)
            }
        },
        Some("lock") => match cmd_lock() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("rvpm: {}", e);
                ExitCode::from(1)
            }
        },
        Some("add") => run(cmd_add(&args[1..])),
        Some("install") => run(cmd_install(&args[1..])),
        Some("update") => run(cmd_update(&args[1..])),
        Some(other) => {
            eprintln!("rvpm: unknown subcommand '{}'", other);
            eprintln!("Run 'rvpm help' for usage.");
            ExitCode::from(1)
        }
    }
}

fn cmd_init(args: &[String]) -> Result<(), InitError> {
    let cwd = std::env::current_dir().map_err(InitError::Io)?;
    let name = match args.first() {
        Some(n) => n.clone(),
        None => name_from_dir(&cwd),
    };
    let dir = PathBuf::from(".");
    init_project(&dir, &name)?;
    println!("Created package '{}'", name);
    println!("  rv.toml");
    println!("  src/main.rv");
    Ok(())
}

fn cmd_fetch(args: &[String]) -> Result<(), String> {
    let spec = args
        .first()
        .ok_or_else(|| "fetch needs a 'github.com/<user>/<repo>@<version>' argument".to_string())?;
    let (path, version) = spec
        .rsplit_once('@')
        .ok_or_else(|| format!("'{}' is missing an '@<version>' suffix", spec))?;
    let gh = GithubPath::parse(path)
        .ok_or_else(|| format!("'{}' is not a 'github.com/<user>/<repo>' path", path))?;
    let dir = pkg::fetch(&gh.host, &gh.user, &gh.repo, version).map_err(|e| e.to_string())?;
    println!("{}", dir.display());
    Ok(())
}

/// Generate or validate `rv.lock` for the package in the current
/// directory. Generates when the lock is absent or does not cover every
/// dependency in `rv.toml`; otherwise validates the existing lock against
/// the cache. The full install/build UX lands in later releases; this is a
/// way to exercise resolution and validation directly.
fn cmd_lock() -> Result<(), String> {
    let manifest = Manifest::load("rv.toml").map_err(|e| e.to_string())?;
    let lock_path = std::path::Path::new(LOCK_FILE_NAME);

    if lock_path.exists() {
        let existing = LockFile::load(lock_path).map_err(|e| e.to_string())?;
        if existing.covers(&manifest) {
            lock::validate_lock(&existing).map_err(|e| e.to_string())?;
            println!("rv.lock is up to date and verified");
            return Ok(());
        }
    }

    let lock = lock::resolve_and_lock(&manifest).map_err(|e| e.to_string())?;
    lock.write(lock_path).map_err(|e| e.to_string())?;
    println!(
        "Wrote {} with {} package(s)",
        LOCK_FILE_NAME,
        lock.packages.len()
    );
    Ok(())
}

/// Print a command's outcome lines, mapping its error to a non-zero exit.
fn run(result: Result<Vec<String>, String>) -> ExitCode {
    match result {
        Ok(lines) => {
            for line in lines {
                println!("{}", line);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("rvpm: {}", e);
            ExitCode::from(1)
        }
    }
}

/// Append (or update) a dependency in rv.toml, then resolve and write
/// rv.lock. Accepts `github.com/<user>/<repo>` with an optional
/// `@<version>`; without a version a placeholder constraint is recorded.
fn cmd_add(args: &[String]) -> Result<Vec<String>, String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(vec![add_usage()]);
    }
    let spec = args
        .first()
        .ok_or_else(|| format!("add needs a package argument\n{}", add_usage()))?;
    let (path, version) = match spec.rsplit_once('@') {
        Some((p, v)) => (p, Some(v)),
        None => (spec.as_str(), None),
    };
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let report = ops::add(&cwd, path, version).map_err(|e| e.to_string())?;
    Ok(report.outcome_lines)
}

/// Re-resolve rv.toml against rv.lock and fill the cache, validating an
/// existing lock or writing a fresh one.
fn cmd_install(args: &[String]) -> Result<Vec<String>, String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(vec![install_usage()]);
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let (_outcome, report) = ops::install(&cwd).map_err(|e| e.to_string())?;
    Ok(report.outcome_lines)
}

/// Re-resolve rv.toml and rewrite rv.lock, for one named package or all.
fn cmd_update(args: &[String]) -> Result<Vec<String>, String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(vec![update_usage()]);
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let package = args.first().map(String::as_str);
    let report = ops::update(&cwd, package).map_err(|e| e.to_string())?;
    Ok(report.outcome_lines)
}

fn add_usage() -> String {
    "Usage: rvpm add github.com/<user>/<repo>[@<version>]".to_string()
}

fn install_usage() -> String {
    "Usage: rvpm install".to_string()
}

fn update_usage() -> String {
    "Usage: rvpm update [github.com/<user>/<repo>]".to_string()
}

fn print_usage() {
    println!("rvpm: the Raven package manager");
    println!();
    println!("Usage:");
    println!("  rvpm <command> [arguments]");
    println!();
    println!("Commands:");
    println!("  init [name]    Scaffold a new package in the current directory");
    println!("  add <pkg>      Add a dependency to rv.toml, then resolve and write rv.lock");
    println!("  install        Resolve rv.toml against rv.lock and fill the cache");
    println!("  update [pkg]   Re-resolve rv.toml and rewrite rv.lock for one package or all");
    println!("  fetch <pkg>    Fetch 'github.com/<user>/<repo>@<version>' into the shared cache");
    println!("  lock           Generate or validate rv.lock for the current package");
    println!("  help           Print this message");
    println!();
    println!("Package arguments use the 'github.com/<user>/<repo>' form.");
    println!("For 'add', append '@<version>' to pin a git tag or branch; without it");
    println!("a placeholder constraint is recorded. build and run land in a later release.");
}
