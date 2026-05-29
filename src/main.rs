//! Raven v2 compiler entry point.
//!
//! Supports two modes:
//!   raven build <source.rv> [-o <output>]
//!     Compile a single source file to a native executable.
//!   raven                            (no args)
//!     Print a placeholder banner. A full REPL lands later.
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
    if args.len() == 1 {
        eprintln!("Raven v2: under construction. See docs/v2/ for the roadmap.");
        return ExitCode::SUCCESS;
    }
    match args[1].as_str() {
        "build" => match run_build(&args[2..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("raven: {}", e);
                ExitCode::from(1)
            }
        },
        other => {
            eprintln!(
                "raven: unknown subcommand `{}`. Try `raven build <file.rv> -o <output>`.",
                other
            );
            ExitCode::from(2)
        }
    }
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
