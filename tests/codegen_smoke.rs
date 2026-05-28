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
use raven::resolve::{expand_with_stdlib, resolve_file, FsLoader};
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

#[test]
fn closure_capture_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // make_adder(10) returns a closure capturing the local `n = 10` by
    // value. Invoking the returned closure value adds the captured amount:
    // add10(5) prints 15 and add10(32) prints 42.
    compile_link_run_and_check("closure_capture.rv", "15\n42\n", &runtime);
}

#[test]
fn closure_arg_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A capturing closure passed as a function argument and invoked
    // indirectly: `triple` captures `factor = 3`, and apply(triple, 7)
    // returns 7 * 3, printing 21.
    compile_link_run_and_check("closure_arg.rv", "21\n", &runtime);
}

#[test]
fn dyn_dispatch_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Two concrete types are coerced to `dyn Speak` and dispatched
    // through their vtables: Dog.sound() is 1, Cat.sound() is 2, each on
    // its own line.
    compile_link_run_and_check("dyn_dispatch.rv", "1\n2\n", &runtime);
}

#[test]
fn interpolation_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // String interpolation end to end: a String fragment (`name`) and an
    // arithmetic Int fragment (`a + b`) are converted, concatenated, and
    // printed. Prints `Hello, Raven!` then `sum is 7`.
    compile_link_run_and_check("interpolation.rv", "Hello, Raven!\nsum is 7\n", &runtime);
}

#[test]
fn defer_order_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Two defers run in reverse declaration order at the return: the
    // function schedules print_int(1) then print_int(2), so the program
    // prints 2 then 1.
    compile_link_run_and_check("defer_order.rv", "2\n1\n", &runtime);
}

#[test]
fn defer_early_return_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // Only reached defers run. f(true) takes the early return after the
    // first defer, printing 9. f(false) reaches both defers and runs
    // them LIFO at its return, printing 8 then 9. Combined: 9, 8, 9.
    compile_link_run_and_check("defer_early_return.rv", "9\n8\n9\n", &runtime);
}

#[test]
fn ffi_strlen_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // C FFI end to end: `extern "C" { fun strlen(s: CStr) -> CSize }` is
    // declared as an imported symbol, resolved against the CRT at link
    // time, and called on the C string literal `c"hello"` (a static
    // null-terminated buffer). strlen counts 5 bytes; prints 5.
    compile_link_run_and_check("ffi_strlen.rv", "5\n", &runtime);
}

#[test]
fn ffi_abs_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A non-pointer FFI type: `abs(x: CInt) -> CInt` takes and returns a
    // 32-bit C int. The negative literal -7 is reduced to i32 at the
    // call; abs(-7) is 7.
    compile_link_run_and_check("ffi_abs.rv", "7\n", &runtime);
}

#[test]
fn std_io_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // The first stdlib module end to end: `import std/io { println,
    // println_int }` merges the bundled `std/io` source into the program,
    // namespaced as `std.io.*`. The selectors bind to those functions,
    // which call the internal `__io_*` intrinsics wired to the runtime.
    // Prints the greeting then 42.
    compile_link_run_and_check("use_io.rv", "Hello from std/io!\n42\n", &runtime);
}

#[test]
fn std_string_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // The std/string module end to end: `import std/string { ... }` merges
    // the bundled source (which builds its utilities in pure Raven on top
    // of the `__str_*` byte intrinsics) into the program, namespaced as
    // `std.string.*`. Exercises case mapping, trim, repeat, replace,
    // substring, contains, and index_of, with the last two observed
    // through println and println_int.
    let expected = "HELLO\n\
                    world\n\
                    spaced\n\
                    ababab\n\
                    a+b+c\n\
                    ave\n\
                    contains: yes\n\
                    2\n\
                    -1\n";
    compile_link_run_and_check("use_string.rv", expected, &runtime);
}

#[test]
fn builtin_method_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A method declared on the built in `Int` type via `impl Int { fun
    // doubled(self) -> Int = self * 2 }`. The call `21.doubled()`
    // resolves to the impl method and dispatches statically to the per
    // type symbol `Int$doubled`, printing 42.
    compile_link_run_and_check("builtin_method.rv", "42\n", &runtime);
}

#[test]
fn stdlib_string_method_program_compiles_and_runs() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    // A method declared on the built in `String` type inside the bundled
    // std/string module: `impl String { fun shout(self) -> String }`.
    // Importing the module merges the `impl` block into the program; the
    // method is resolved by the receiver's type, not by an imported name.
    // `shout` calls the module's sibling `to_upper`, so "hi".shout()
    // prints HI.
    compile_link_run_and_check("stdlib_string_method.rv", "HI\n", &runtime);
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
    // Mirror the driver: merge any imported bundled stdlib modules before
    // resolving. A program with no `std/` imports is unchanged.
    let file = expand_with_stdlib(&file).map_err(|e| format!("stdlib: {}", e))?;
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
