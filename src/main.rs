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

use std::ffi::{OsStr, OsString};
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
    // Collect arguments with `args_os` rather than `args`. On Unix an argument
    // is an arbitrary byte string, and `args` panics when one is not valid
    // UTF-8, so a non-UTF-8 source path would crash the compiler before it
    // could report an ordinary diagnostic. Source paths flow through as
    // `OsString`, preserving the bytes; only the subcommand is matched as text.
    let args: Vec<OsString> = std::env::args_os().collect();
    let Some(first) = args.get(1) else {
        print_usage();
        return ExitCode::SUCCESS;
    };
    match first.to_str() {
        Some("help") | Some("--help") | Some("-h") => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some("--version") | Some("-V") => {
            print_version();
            ExitCode::SUCCESS
        }
        Some("build") => match run_build(&args[2..]) {
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
        _ => {
            eprintln!("raven: unknown subcommand '{}'", first.to_string_lossy());
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

fn run_build(rest: &[OsString]) -> Result<(), BuildError> {
    let opts = parse_build_args(rest)?;
    // Refuse to write the executable over the input source. The compiler reads
    // the source first and the linker writes the output last, so `-o` pointing
    // at the source would silently replace it with the binary; a typo there
    // could destroy the only copy of the file.
    if same_file(&opts.input, &opts.output) {
        return Err(BuildError::Args(format!(
            "refusing to overwrite the input source `{}` with the build output; pass a different -o path",
            opts.input.display()
        )));
    }
    // The single-file `raven build` has no package context, so external
    // (`github.com/...`) imports stay deferred and surface as unresolved.
    // Package-aware builds go through `rvpm build`.
    driver::build_binary(&opts.input, &opts.output, None).map_err(BuildError::Driver)
}

/// Whether two paths refer to the same file. Canonicalization resolves `.`,
/// `..`, symlinks, and case differences; when the output does not exist yet it
/// cannot be the input, so the paths are compared as written.
fn same_file(input: &Path, output: &Path) -> bool {
    match (std::fs::canonicalize(input), std::fs::canonicalize(output)) {
        (Ok(a), Ok(b)) => a == b,
        _ => input == output,
    }
}

#[derive(Debug)]
struct BuildOpts {
    input: PathBuf,
    output: PathBuf,
}

/// Whether an argument is an option flag, i.e. begins with `-`. The leading
/// byte is checked directly so the test works for arguments that are not valid
/// UTF-8 (a non-UTF-8 source path is treated as a positional, not a flag).
fn is_flag(arg: &OsStr) -> bool {
    arg.as_encoded_bytes().first() == Some(&b'-')
}

fn parse_build_args(args: &[OsString]) -> Result<BuildOpts, BuildError> {
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
            // The path keeps its original bytes; a non-UTF-8 path then surfaces
            // as an ordinary "no such file" diagnostic from the driver.
            output = Some(PathBuf::from(&args[i]));
        } else if is_flag(a) {
            return Err(BuildError::Args(format!(
                "unknown flag `{}`",
                a.to_string_lossy()
            )));
        } else if input.is_none() {
            input = Some(PathBuf::from(a));
        } else {
            return Err(BuildError::Args(format!(
                "unexpected positional argument `{}`",
                a.to_string_lossy()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_file_detects_the_input_as_output() {
        // A per-process directory so parallel test runs do not collide.
        let dir = std::env::temp_dir().join(format!("raven_same_file_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("prog.rv");
        std::fs::write(&src, "fun main() {}\n").unwrap();

        // The output pointing at the existing source is the same file, even
        // when written with a `.` segment.
        assert!(same_file(&src, &src));
        assert!(same_file(&src, &dir.join(".").join("prog.rv")));

        // A distinct (non-existent) output path is not the input.
        assert!(!same_file(&src, &dir.join("prog.exe")));

        std::fs::remove_dir_all(&dir).ok();
    }

    // A non-UTF-8 source path must reach the build pipeline with its bytes
    // intact instead of panicking while the arguments are collected.
    #[cfg(unix)]
    #[test]
    fn build_args_keep_non_utf8_input_path() {
        use std::os::unix::ffi::OsStringExt;

        let input = OsString::from_vec(b"bad-\xff.rv".to_vec());
        let opts = parse_build_args(&[input.clone()]).expect("non-UTF-8 path parses");
        assert_eq!(opts.input.as_os_str(), input.as_os_str());
    }
}
