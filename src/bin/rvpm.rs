//! The rvpm package manager command line entry point.
//!
//! rvpm manages Raven packages described by an `rv.toml` manifest. This
//! binary is intentionally thin: it parses arguments and dispatches to
//! library code in `raven::manifest`. Today only `init` is implemented.

use std::path::PathBuf;
use std::process::ExitCode;

use raven::manifest::init::{init_project, name_from_dir, InitError};
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

fn print_usage() {
    println!("rvpm: the Raven package manager");
    println!();
    println!("Usage:");
    println!("  rvpm <command> [arguments]");
    println!();
    println!("Commands:");
    println!("  init [name]   Scaffold a new package in the current directory");
    println!("  fetch <pkg>   Fetch 'github.com/<user>/<repo>@<version>' into the shared cache");
    println!("  help          Print this message");
    println!();
    println!("Dependency resolution, add/install/update, and build/run land in later releases.");
}
