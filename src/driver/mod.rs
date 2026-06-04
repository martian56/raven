//! Compile pipeline orchestration shared by the `raven` and `rvpm`
//! binaries.
//!
//! The `raven` binary compiles a single source file; `rvpm build` compiles
//! a package entry file with a package context so external
//! (`github.com/...`) imports resolve through the rvpm cache. Both drive
//! the same pipeline here: lex, parse, expand (bundled stdlib plus local
//! and external modules), resolve, type check, lower to HIR then MIR, emit
//! an object with Cranelift, and link it with the `raven-runtime`
//! staticlib.
//!
//! The single difference between the two is the optional
//! [`PackageContext`]: with it, external imports are read from the cache;
//! without it (the plain `raven build` path), an external import stays
//! deferred and surfaces as an unresolved import, exactly as before.

use std::path::{Path, PathBuf};

use crate::codegen::linker::{self, RuntimeStaticLib};
use crate::codegen::{self, CodegenError};
use crate::hir::lower_file;
use crate::lexer::Lexer;
use crate::mir::lower_program;
use crate::parser::parse_with_macros;
use crate::resolve::{expand_with_stdlib_ctx, resolve_file_ctx, FsLoader, PackageContext};
use crate::tycheck::check_file;

/// An error from the compile pipeline or the link step.
#[derive(Debug)]
pub enum DriverError {
    Io(String),
    Frontend(String),
    /// A fully rendered, multi-line source diagnostic (headline, pointer,
    /// help/notes). Printed verbatim, without the `raven:` prefix.
    Diagnostic(String),
    Codegen(CodegenError),
    RuntimeMissing,
}

/// Render a front-end [`RavenError`] into a [`DriverError::Diagnostic`],
/// reading the offending span's file when it differs from the entry file
/// (for example an error inside a local module or a dependency).
fn frontend_diag(e: crate::error::RavenError, input: &Path, source: &str) -> DriverError {
    let span_file = e.span().file.clone();
    let src: std::borrow::Cow<str> = if span_file.as_path() == input {
        std::borrow::Cow::Borrowed(source)
    } else {
        match std::fs::read_to_string(span_file.as_path()) {
            Ok(s) => std::borrow::Cow::Owned(s),
            Err(_) => std::borrow::Cow::Borrowed(source),
        }
    };
    DriverError::Diagnostic(e.display(&src))
}

impl From<CodegenError> for DriverError {
    fn from(e: CodegenError) -> Self {
        DriverError::Codegen(e)
    }
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverError::Io(s) => write!(f, "io: {}", s),
            DriverError::Frontend(s) => write!(f, "frontend: {}", s),
            DriverError::Diagnostic(s) => f.write_str(s),
            DriverError::Codegen(e) => write!(f, "{}", e),
            DriverError::RuntimeMissing => f.write_str(
                "could not locate raven_runtime staticlib; build it with `cargo build -p raven-runtime` or set RAVEN_RUNTIME_LIB",
            ),
        }
    }
}

impl std::error::Error for DriverError {}

/// Compile `input` to a native executable at `output`.
///
/// When `ctx` is `Some`, external (`github.com/...`) imports in the program
/// (and its local and external modules) resolve through the rvpm cache.
/// When `None`, the pipeline behaves exactly as a single-file `raven build`.
pub fn build_binary(
    input: &Path,
    output: &Path,
    ctx: Option<&PackageContext>,
) -> Result<(), DriverError> {
    let source = std::fs::read_to_string(input)
        .map_err(|e| DriverError::Io(format!("read {}: {}", input.display(), e)))?;

    let object_bytes = compile_to_object(&source, input, ctx)?;

    let runtime = locate_runtime_staticlib()?;
    let tmp = TempDir::new()?;
    let object_path = tmp.path().join("raven_program.o");
    std::fs::write(&object_path, &object_bytes)
        .map_err(|e| DriverError::Io(format!("write object: {}", e)))?;

    linker::link(&object_path, &runtime, output).map_err(DriverError::from)?;
    Ok(())
}

/// Run the front and middle ends and Cranelift to produce a relocatable
/// object for `source`. Threads `ctx` through expansion and resolution.
pub fn compile_to_object(
    source: &str,
    input: &Path,
    ctx: Option<&PackageContext>,
) -> Result<Vec<u8>, DriverError> {
    let tokens = Lexer::new(source.to_string(), input.to_path_buf())
        .tokenize()
        .map_err(|e| frontend_diag(e, input, source))?;
    // Collect the file's macro table from the original token stream (before
    // the definitions are stripped) so a macro call inside a `"${...}"`
    // interpolation fragment can be expanded while that fragment is parsed.
    let macro_table =
        crate::macros::collect_macro_table(&tokens).map_err(|e| frontend_diag(e, input, source))?;
    let tokens =
        crate::macros::expand_tokens(&tokens).map_err(|e| frontend_diag(e, input, source))?;
    let file =
        parse_with_macros(&tokens, macro_table).map_err(|e| frontend_diag(e, input, source))?;
    let file = expand_with_stdlib_ctx(&file, ctx).map_err(|e| frontend_diag(e, input, source))?;
    let mut loader = FsLoader;
    let resolved =
        resolve_file_ctx(&file, &mut loader, ctx).map_err(|e| frontend_diag(e, input, source))?;
    let typed = check_file(&resolved).map_err(|e| frontend_diag(e, input, source))?;
    let hir = lower_file(&typed).map_err(|e| frontend_diag(e, input, source))?;
    if std::env::var("RAVEN_DUMP_HIR").is_ok() {
        eprintln!("{}", crate::hir::pretty_program(&hir));
    }
    let mir = lower_program(&hir).map_err(|e| frontend_diag(e, input, source))?;
    if std::env::var("RAVEN_DUMP_MIR").is_ok() {
        eprintln!("{}", crate::mir::pretty::pretty_program(&mir));
    }
    codegen::compile_program(&mir).map_err(DriverError::from)
}

/// Locate the `raven-runtime` staticlib next to the compiler binary, in
/// the current directory's `target/`, or via `RAVEN_RUNTIME_LIB`.
pub fn locate_runtime_staticlib() -> Result<RuntimeStaticLib, DriverError> {
    if let Ok(p) = std::env::var("RAVEN_RUNTIME_LIB") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Ok(RuntimeStaticLib { path: pb });
        }
    }
    for c in candidate_runtime_paths() {
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

/// A temporary directory removed on drop, used to hold the intermediate
/// object file.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Result<TempDir, DriverError> {
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

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
