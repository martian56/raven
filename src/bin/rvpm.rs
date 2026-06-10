//! The rvpm package manager command line entry point.
//!
//! rvpm manages Raven packages described by an `rv.toml` manifest. This
//! binary is intentionally thin: it parses arguments and dispatches to
//! library code in `raven::manifest`, `raven::pkg`, `raven::lock`, and
//! `raven::ops`.

use std::path::PathBuf;
use std::process::ExitCode;

use raven::format::format_source_with;
use raven::lock::{self, LockFile, LOCK_FILE_NAME};
use raven::manifest::init::{init_project, name_from_dir, InitError};
use raven::manifest::Manifest;
use raven::ops;
use raven::pkg;
use raven::resolve::GithubPath;

/// `rvpm build`/`run` invoke the compiler, which recurses deeply (derive
/// expansion, type checking), enough to overflow the default main-thread stack
/// in debug builds. Run the work on a thread with a generous stack, the same as
/// the `raven` CLI. The reservation is virtual address space; the OS commits
/// pages only as they are touched.
const COMPILER_STACK_SIZE: usize = 512 * 1024 * 1024;

fn main() -> ExitCode {
    let worker = std::thread::Builder::new()
        .stack_size(COMPILER_STACK_SIZE)
        .spawn(dispatch)
        .expect("spawn rvpm worker thread");
    match worker.join() {
        Ok(code) => code,
        Err(_) => ExitCode::from(101),
    }
}

fn dispatch() -> ExitCode {
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
        Some("build") => run(cmd_build(&args[1..])),
        Some("run") => cmd_run(&args[1..]),
        Some("fmt") => cmd_fmt(&args[1..]),
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

/// Build the package in the current directory: ensure dependencies are
/// installed, then compile `src/main.rv` to `target/raven-out/<name>`.
fn cmd_build(args: &[String]) -> Result<Vec<String>, String> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return Ok(vec![build_usage()]);
    }
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let report = ops::build(&cwd).map_err(|e| e.to_string())?;
    Ok(report.outcome_lines)
}

/// Build the package then run the produced binary, forwarding any args
/// after `run` to the program and exiting with its code.
fn cmd_run(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", run_usage());
        return ExitCode::SUCCESS;
    }
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("rvpm: {}", e);
            return ExitCode::from(1);
        }
    };
    match ops::run_package(&cwd, args) {
        Ok(code) => {
            let code: u8 = code.try_into().unwrap_or(1);
            ExitCode::from(code)
        }
        Err(e) => {
            eprintln!("rvpm: {}", e);
            ExitCode::from(1)
        }
    }
}

/// Format Raven sources in place, or check formatting with `--check`.
///
/// With no path arguments, formats every `.rv` file under the project
/// `src/` directory. With `--check`, no file is written: the command lists
/// files that are not canonically formatted and exits non-zero if any are.
fn cmd_fmt(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", fmt_usage());
        return ExitCode::SUCCESS;
    }
    let check = args.iter().any(|a| a == "--check");
    let paths: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();

    let files = if paths.is_empty() {
        match collect_rv_files(PathBuf::from("src")) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("rvpm: {}", e);
                return ExitCode::from(1);
            }
        }
    } else {
        let mut acc = Vec::new();
        for p in &paths {
            let path = PathBuf::from(p);
            if path.is_dir() {
                match collect_rv_files(path) {
                    Ok(mut f) => acc.append(&mut f),
                    Err(e) => {
                        eprintln!("rvpm: {}", e);
                        return ExitCode::from(1);
                    }
                }
            } else {
                acc.push(path);
            }
        }
        acc
    };

    if files.is_empty() {
        eprintln!("rvpm: no .rv files to format");
        return ExitCode::from(1);
    }

    // Honor `[fmt].indent_width` from the project manifest when there is one;
    // fall back to the default outside a project.
    let indent_width = Manifest::load("rv.toml")
        .map(|m| m.fmt.indent_width)
        .unwrap_or(4);

    let mut changed: Vec<PathBuf> = Vec::new();
    let mut errored = false;
    for path in &files {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("rvpm: {}: {}", path.display(), e);
                errored = true;
                continue;
            }
        };
        let formatted = match format_source_with(&src, indent_width) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("rvpm: {}: {}", path.display(), e);
                errored = true;
                continue;
            }
        };
        if formatted == src {
            continue;
        }
        if check {
            changed.push(path.clone());
        } else if let Err(e) = std::fs::write(path, &formatted) {
            eprintln!("rvpm: {}: {}", path.display(), e);
            errored = true;
        } else {
            println!("formatted {}", path.display());
        }
    }

    if errored {
        return ExitCode::from(1);
    }
    if check {
        if changed.is_empty() {
            return ExitCode::SUCCESS;
        }
        eprintln!("The following files are not formatted:");
        for p in &changed {
            eprintln!("  {}", p.display());
        }
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// Recursively collect `.rv` files under `dir`, sorted for deterministic
/// output.
fn collect_rv_files(dir: PathBuf) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() {
        return Err(format!("'{}' does not exist", dir.display()));
    }
    let mut out = Vec::new();
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {}", d.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rv") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn fmt_usage() -> String {
    "Usage: rvpm fmt [--check] [paths...]".to_string()
}

fn build_usage() -> String {
    "Usage: rvpm build".to_string()
}

fn run_usage() -> String {
    "Usage: rvpm run [program arguments]".to_string()
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
    println!("  build          Compile src/main.rv to target/raven-out/<name>");
    println!("  run [args]     Build the package then run it, forwarding args");
    println!("  fmt [paths]    Format .rv files in place (--check to verify only)");
    println!("  fetch <pkg>    Fetch 'github.com/<user>/<repo>@<version>' into the shared cache");
    println!("  lock           Generate or validate rv.lock for the current package");
    println!("  help           Print this message");
    println!();
    println!("Package arguments use the 'github.com/<user>/<repo>' form.");
    println!("For 'add', append '@<version>' to pin a git tag or branch; without it");
    println!("a placeholder constraint is recorded.");
}
