//! End to end smoke test for the Cranelift back end.
//!
//! Compiles `examples/v2/hello.rv` with the driver, links the
//! resulting object with the `raven-runtime` staticlib using the
//! toolchain-aware linker (MSVC `link.exe` on windows-msvc, `cc`
//! elsewhere), runs the binary, and checks that stdout matches
//! `Hello, Raven!\n`. On a correctly configured host the test links and
//! runs the program for real. It short circuits with a diagnostic on
//! `eprintln!` and a successful exit only in the genuinely unsupported
//! cases: no linker is available at all, or the runtime staticlib has
//! not been built yet.

use std::path::{Path, PathBuf};
use std::process::Command;

use raven::codegen::linker::{self, RuntimeStaticLib};
use raven::codegen::{self, CodegenError};
use raven::hir::lower_file;
use raven::lexer::Lexer;
use raven::mir::lower_program;
use raven::parser::parse;
use raven::resolve::{resolve_file, FsLoader};
use raven::tycheck::check_file;

#[test]
fn hello_world_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    compile_link_run_and_check("hello.rv", "Hello, Raven!\n", &runtime);
}

#[test]
fn struct_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Point { x: 3, y: 4 } built on the heap, passed by reference, and
    // summed through field access: prints 7.
    compile_link_run_and_check("point.rv", "7\n", &runtime);
}

#[test]
fn enum_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Option values matched and unwrapped: 5 + 99 prints 104.
    compile_link_run_and_check("option_sum.rv", "104\n", &runtime);
}

#[test]
fn closure_value_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A non-capturing lambda is allocated as a closure object; the
    // program prints 42 to show the allocation and GC root frame run.
    compile_link_run_and_check("closure_value.rv", "42\n", &runtime);
}

/// Return the runtime staticlib when a linker and the staticlib are both
/// present, or skip with a diagnostic. Shared by every smoke case so the
/// skip behavior stays identical.
fn supported_runtime() -> Option<RuntimeStaticLib> {
    if !linker::linker_available() {
        eprintln!(
            "codegen_smoke: skipping, no linker available for the host. \
             Install the MSVC C++ build tools on windows-msvc, a 64-bit \
             MinGW-w64 on windows-gnu, or a `cc` driver on Unix."
        );
        return None;
    }
    match locate_runtime() {
        Some(r) => Some(r),
        None => {
            eprintln!(
                "codegen_smoke: skipping, raven_runtime staticlib not built. \
                 Run `cargo build -p raven-runtime` to produce it."
            );
            None
        }
    }
}

/// Compile `examples/v2/<name>`, link it with the runtime, run it, and
/// assert its stdout equals `expected`. Panics on any failure on a
/// supported host so a regression is loud.
fn compile_link_run_and_check(name: &str, expected: &str, runtime: &RuntimeStaticLib) {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2")
        .join(name);
    let source =
        std::fs::read_to_string(&source_path).unwrap_or_else(|e| panic!("read {}: {}", name, e));

    let object_bytes = match build_object(&source, &source_path) {
        Ok(b) => b,
        Err(e) => panic!("frontend or codegen failed for {}: {}", name, e),
    };

    let tmp = workdir();
    let stem = Path::new(name).file_stem().unwrap().to_string_lossy();
    let object_path = tmp.join(format!("{}.o", stem));
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) {
        format!("{}.exe", stem)
    } else {
        stem.to_string()
    });

    if let Err(e) = linker::link(&object_path, runtime, &binary) {
        cleanup(&tmp);
        panic!("linker failed to produce an executable for {}: {}", name, e);
    }

    let output = Command::new(&binary)
        .output()
        .unwrap_or_else(|e| panic!("run {} binary: {}", name, e));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&tmp);
    assert!(
        output.status.success(),
        "{} binary exited non zero: status={:?} stderr={}",
        name,
        output.status,
        stderr
    );
    assert_eq!(
        stdout, expected,
        "unexpected stdout for {}: {:?}",
        name, stdout
    );
}

fn build_object(source: &str, path: &Path) -> Result<Vec<u8>, String> {
    let tokens = Lexer::new(source.to_string(), path.to_path_buf())
        .tokenize()
        .map_err(|e| format!("lex: {}", e))?;
    let file = parse(&tokens).map_err(|e| format!("parse: {}", e))?;
    let mut loader = FsLoader;
    let resolved = resolve_file(&file, &mut loader).map_err(|e| format!("resolve: {}", e))?;
    let typed = check_file(&resolved).map_err(|e| format!("tycheck: {}", e))?;
    let hir = lower_file(&typed).map_err(|e| format!("hir: {}", e))?;
    let mir = lower_program(&hir).map_err(|e| format!("mir: {}", e))?;
    codegen::compile_program(&mir).map_err(|e: CodegenError| format!("codegen: {}", e))
}

fn workdir() -> PathBuf {
    // A process-wide atomic counter makes each tempdir unique even when
    // several smoke tests run in parallel and start within the same
    // nanosecond, so one test's cleanup never deletes another's binary.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    p.push(format!("raven-smoke-{}-{}-{}", pid, stamp, seq));
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

fn cleanup(p: &Path) {
    let _ = std::fs::remove_dir_all(p);
}

fn locate_runtime() -> Option<RuntimeStaticLib> {
    if let Ok(p) = std::env::var("RAVEN_RUNTIME_LIB") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(RuntimeStaticLib { path: pb });
        }
    }
    let lib_name = if cfg!(windows) {
        "raven_runtime.lib"
    } else {
        "libraven_runtime.a"
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for sub in ["target/debug", "target/release"] {
        let p = root.join(sub).join(lib_name);
        if p.is_file() {
            return Some(RuntimeStaticLib { path: p });
        }
    }
    None
}
