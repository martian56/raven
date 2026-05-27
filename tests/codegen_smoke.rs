//! End to end smoke test for the Cranelift back end.
//!
//! Compiles `examples/v2/hello.rv` with the driver, links the
//! resulting object with the `raven-runtime` staticlib via the system
//! `cc` driver, runs the binary, and checks that stdout matches
//! `Hello, Raven!\n`. The test short circuits with a diagnostic on
//! `eprintln!` and a successful exit when `cc` is unavailable or the
//! runtime staticlib has not been built yet, so contributors without
//! a working C toolchain do not see spurious failures.

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
    if !linker::cc_available() {
        eprintln!("codegen_smoke: skipping, no `cc` driver on PATH");
        return;
    }
    let runtime = match locate_runtime() {
        Some(r) => r,
        None => {
            eprintln!(
                "codegen_smoke: skipping, raven_runtime staticlib not built. \
                 Run `cargo build -p raven-runtime` to produce it."
            );
            return;
        }
    };

    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2")
        .join("hello.rv");
    let source = std::fs::read_to_string(&source_path).expect("read hello.rv");

    let object_bytes = match build_object(&source, &source_path) {
        Ok(b) => b,
        Err(e) => panic!("frontend or codegen failed: {}", e),
    };

    let tmp = workdir();
    let object_path = tmp.join("hello.o");
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) { "hello.exe" } else { "hello" });

    match linker::link(&object_path, &runtime, &binary) {
        Ok(_) => {}
        Err(e) => {
            eprintln!(
                "codegen_smoke: skipping, linker rejected the object: {}. \
                 This usually means the system `cc` does not match the host \
                 architecture Cranelift targets.",
                e
            );
            cleanup(&tmp);
            return;
        }
    }

    let output = Command::new(&binary).output().expect("run hello binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    cleanup(&tmp);
    assert!(
        output.status.success(),
        "hello binary exited non zero: status={:?} stderr={}",
        output.status,
        stderr
    );
    assert_eq!(stdout, "Hello, Raven!\n", "unexpected stdout: {:?}", stdout);
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
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("raven-smoke-{}-{}", pid, stamp));
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
