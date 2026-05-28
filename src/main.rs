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
//! (MSVC `link.exe` on windows-msvc, `cc` elsewhere).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use raven::codegen::linker::{self, RuntimeStaticLib};
use raven::codegen::{self, CodegenError};
use raven::hir::lower_file;
use raven::lexer::Lexer;
use raven::mir::lower_program;
use raven::parser::parse;
use raven::resolve::{resolve_file, FsLoader};
use raven::tycheck::check_file;

fn main() -> ExitCode {
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

fn run_build(rest: &[String]) -> Result<(), DriverError> {
    let opts = parse_build_args(rest)?;
    let source = std::fs::read_to_string(&opts.input)
        .map_err(|e| DriverError::Io(format!("read {}: {}", opts.input.display(), e)))?;

    // Front end.
    let tokens = Lexer::new(source.clone(), opts.input.clone())
        .tokenize()
        .map_err(|e| DriverError::Frontend(format!("lex: {}", e)))?;
    let file = parse(&tokens).map_err(|e| DriverError::Frontend(format!("parse: {}", e)))?;
    let mut loader = FsLoader;
    let resolved = resolve_file(&file, &mut loader)
        .map_err(|e| DriverError::Frontend(format!("resolve: {}", e)))?;
    let typed =
        check_file(&resolved).map_err(|e| DriverError::Frontend(format!("tycheck: {}", e)))?;
    let hir = lower_file(&typed).map_err(|e| DriverError::Frontend(format!("hir: {}", e)))?;
    let mir = lower_program(&hir).map_err(|e| DriverError::Frontend(format!("mir: {}", e)))?;

    // Back end.
    let object_bytes = codegen::compile_program(&mir).map_err(DriverError::from)?;

    let tmp = tempdir_for_build()?;
    let object_path = tmp.path().join("raven_program.o");
    std::fs::write(&object_path, &object_bytes)
        .map_err(|e| DriverError::Io(format!("write object: {}", e)))?;

    // Locate the runtime staticlib. The driver looks for it in the
    // standard Cargo target directory next to the compiler binary, the
    // current working directory's target, and an optional override via
    // the RAVEN_RUNTIME_LIB environment variable.
    let runtime = locate_runtime_staticlib()?;

    linker::link(&object_path, &runtime, &opts.output).map_err(DriverError::from)?;

    // Object file lives in a tempdir that is cleaned up on drop.
    drop(tmp);
    Ok(())
}

#[derive(Debug)]
struct BuildOpts {
    input: PathBuf,
    output: PathBuf,
}

fn parse_build_args(args: &[String]) -> Result<BuildOpts, DriverError> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-o" || a == "--output" {
            i += 1;
            if i >= args.len() {
                return Err(DriverError::Args("expected an output path after -o".into()));
            }
            output = Some(PathBuf::from(&args[i]));
        } else if a.starts_with("-") {
            return Err(DriverError::Args(format!("unknown flag `{}`", a)));
        } else if input.is_none() {
            input = Some(PathBuf::from(a));
        } else {
            return Err(DriverError::Args(format!(
                "unexpected positional argument `{}`",
                a
            )));
        }
        i += 1;
    }
    let input = input.ok_or_else(|| DriverError::Args("missing input source file".into()))?;
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

fn locate_runtime_staticlib() -> Result<RuntimeStaticLib, DriverError> {
    if let Ok(p) = std::env::var("RAVEN_RUNTIME_LIB") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Ok(RuntimeStaticLib { path: pb });
        }
    }
    // Search relative to the compiler binary and the current directory.
    let candidates = candidate_runtime_paths();
    for c in candidates {
        if c.is_file() {
            return Ok(RuntimeStaticLib { path: c });
        }
    }
    Err(DriverError::RuntimeMissing)
}

fn candidate_runtime_paths() -> Vec<PathBuf> {
    let lib_name = if cfg!(windows) {
        "raven_runtime.lib"
    } else {
        "libraven_runtime.a"
    };
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.parent().into_iter().flat_map(|d| {
            [
                d.to_path_buf(),
                d.join("deps"),
                d.parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or(d.to_path_buf()),
            ]
        }) {
            out.push(dir.join(lib_name));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for sub in [
            "target/debug",
            "target/release",
            "target\\debug",
            "target\\release",
        ] {
            out.push(cwd.join(sub).join(lib_name));
        }
    }
    out
}

/// Create a temporary directory the driver uses to hold the
/// intermediate object file. The struct removes the directory on drop.
fn tempdir_for_build() -> Result<TempDir, DriverError> {
    let mut base = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    base.push(format!("raven-build-{}-{}", pid, stamp));
    std::fs::create_dir_all(&base)
        .map_err(|e| DriverError::Io(format!("mkdir {}: {}", base.display(), e)))?;
    Ok(TempDir { path: base })
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug)]
enum DriverError {
    Args(String),
    Io(String),
    Frontend(String),
    Codegen(CodegenError),
    RuntimeMissing,
}

impl From<CodegenError> for DriverError {
    fn from(e: CodegenError) -> Self {
        DriverError::Codegen(e)
    }
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverError::Args(s) => write!(f, "{}", s),
            DriverError::Io(s) => write!(f, "io: {}", s),
            DriverError::Frontend(s) => write!(f, "frontend: {}", s),
            DriverError::Codegen(e) => write!(f, "{}", e),
            DriverError::RuntimeMissing => f.write_str(
                "could not locate raven_runtime staticlib; build it with `cargo build -p raven-runtime` or set RAVEN_RUNTIME_LIB",
            ),
        }
    }
}
