//! Raven v2 compiler entry point.
//!
//! Supports:
//!   raven build <source.rv> [-o <output>]
//!     Compile a single source file to a native executable.
//!   raven help | --help | -h     Print usage.
//!   raven --version | -V         Print the compiler version.
//!   raven                        Print usage.
//!
//! The `build` subcommand runs the entire v2 pipeline (lex, parse,
//! resolve, type check, HIR, MIR) and feeds the resulting `MirProgram`
//! to the Cranelift back end. The relocatable object is then linked
//! with the `raven-runtime` staticlib by the toolchain-aware linker
//! (MSVC `link.exe` on windows-msvc, `cc` elsewhere). The reusable
//! pipeline lives in `raven::driver` so `rvpm build` can drive it with a
//! package context.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use raven::driver::{self, DriverError};

/// Stack size for the compiler worker thread.
///
/// Lowering, type checking, and codegen recurse on the structure of the
/// program, so a deeply nested expression drives the recursion as deep
/// as the nesting. The default main-thread stack (1 MB on Windows) is
/// too small for ordinary nesting in debug builds, where stack frames
/// are large. Run the work on a thread with a generous stack instead,
/// the same approach rustc takes. The reservation is virtual address
/// space; the OS commits pages only as they are touched.
const COMPILER_STACK_SIZE: usize = 512 * 1024 * 1024;

fn main() -> ExitCode {
    let worker = std::thread::Builder::new()
        .stack_size(COMPILER_STACK_SIZE)
        .spawn(run)
        .expect("spawn compiler worker thread");
    match worker.join() {
        Ok(code) => code,
        Err(_) => ExitCode::from(101),
    }
}

fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(first) = args.get(1) else {
        print_usage();
        return ExitCode::SUCCESS;
    };
    match first.as_str() {
        "help" | "--help" | "-h" => {
            print_usage();
            ExitCode::SUCCESS
        }
        "--version" | "-V" => {
            print_version();
            ExitCode::SUCCESS
        }
        "build" => match run_build(&args[2..]) {
            Ok(()) => ExitCode::SUCCESS,
            // A rendered source diagnostic prints verbatim; it carries its own
            // `error:` header, so the `raven:` prefix would only get in the way.
            Err(BuildError::Driver(DriverError::Diagnostic(s))) => {
                eprint!("{}", s);
                ExitCode::from(1)
            }
            Err(e) => {
                eprintln!("raven: {}", e);
                ExitCode::from(1)
            }
        },
        other => {
            eprintln!("raven: unknown subcommand '{}'", other);
            eprintln!("Run 'raven help' for usage.");
            ExitCode::from(2)
        }
    }
}

fn print_version() {
    println!("raven {}", env!("CARGO_PKG_VERSION"));
}

fn print_usage() {
    println!("raven: the Raven compiler");
    println!();
    println!("Usage:");
    println!("  raven <command> [arguments]");
    println!();
    println!("Commands:");
    println!("  build <file.rv> [-o <output>]   Compile a source file to a native executable");
    println!("  help                            Print this message");
    println!();
    println!("Options:");
    println!("  -h, --help                      Print this message");
    println!("  -V, --version                   Print the compiler version");
    println!();
    println!("To manage packages, use the 'rvpm' command.");
}

fn run_build(rest: &[String]) -> Result<(), BuildError> {
    let opts = parse_build_args(rest)?;
    // The single-file `raven build` has no package context, so external
    // (`github.com/...`) imports stay deferred and surface as unresolved.
    // Package-aware builds go through `rvpm build`.
    driver::build_binary(&opts.input, &opts.output, None).map_err(BuildError::Driver)
}

#[derive(Debug)]
struct BuildOpts {
    input: PathBuf,
    output: PathBuf,
}

fn parse_build_args(args: &[String]) -> Result<BuildOpts, BuildError> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-o" || a == "--output" {
            i += 1;
            if i >= args.len() {
                return Err(BuildError::Args("expected an output path after -o".into()));
            }
            output = Some(PathBuf::from(&args[i]));
        } else if a.starts_with('-') {
            return Err(BuildError::Args(format!("unknown flag `{}`", a)));
        } else if input.is_none() {
            input = Some(PathBuf::from(a));
        } else {
            return Err(BuildError::Args(format!(
                "unexpected positional argument `{}`",
                a
            )));
        }
        i += 1;
    }
    let input = input.ok_or_else(|| BuildError::Args("missing input source file".into()))?;
    let output = output.unwrap_or_else(|| default_output_for(&input));
    Ok(BuildOpts { input, output })
}

fn default_output_for(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("a")
        .to_string();
    if cfg!(windows) {
        PathBuf::from(format!("{}.exe", stem))
    } else {
        PathBuf::from(stem)
    }
}

#[derive(Debug)]
enum BuildError {
    Args(String),
    Driver(DriverError),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Args(s) => write!(f, "{}", s),
            BuildError::Driver(e) => write!(f, "{}", e),
        }
    }
}
