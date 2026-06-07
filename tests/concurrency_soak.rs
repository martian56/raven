//! Concurrency soak: run parallel goroutine programs many times per build to
//! shake out flaky scheduler/GC races (hangs, wrong output, crashes) that a
//! single run can miss. Each run is a fresh process under an aggressive
//! `RAVEN_GC_THRESHOLD` so collections fire constantly while goroutines run in
//! parallel on the worker pool; a per-run timeout turns a deadlock into a clean
//! failure instead of a CI hang.
//!
//! Two shapes, both with a deterministic result so corruption shows as a wrong
//! number: a parallel reduction (eight allocation-heavy goroutines) and a
//! goroutine spinning a long NON-allocating loop while another allocates hard
//! (the loop-back-edge-safepoint case). The default counts are sized for CI;
//! widen with `RAVEN_SOAK_RUNS`, `RAVEN_SOAK_THRESHOLD`, `RAVEN_SOAK_TIMEOUT_SECS`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use raven::codegen::linker::{self, RuntimeStaticLib};
use raven::codegen::{self, CodegenError};
use raven::hir::lower_file;
use raven::lexer::Lexer;
use raven::mir::lower_program;
use raven::parser::parse;
use raven::resolve::{expand_with_stdlib, resolve_file, FsLoader};
use raven::tycheck::check_file;

const DEFAULT_RUNS: u64 = 50;

/// A goroutine spins a long non-allocating loop (only a safepoint poll keeps it
/// stoppable) while another allocates hard enough to keep the collector busy.
/// Result is `5 + 0 = 5`.
const SPIN_SOURCE: &str = r#"
import std/sync { Channel, channel }

fun main() {
    let done = channel()
    spawn(fun() -> Unit {
        let i = 0
        while i < 5000000 {
            i = i + 1
        }
        done.send(i / 1000000)
    })
    spawn(fun() -> Unit {
        let k = 0
        while k < 20000 {
            let items = [k, k + 1]
            k = k + items.len()
        }
        done.send(0)
    })
    let total = done.recv() + done.recv()
    print(total)
}
"#;

#[test]
fn parallel_programs_survive_a_soak() {
    let Some(runtime) = supported_runtime() else {
        return;
    };
    let runs = env_u64("RAVEN_SOAK_RUNS", DEFAULT_RUNS) as usize;
    let threshold = std::env::var("RAVEN_SOAK_THRESHOLD").unwrap_or_else(|_| "2048".to_string());
    let timeout = Duration::from_secs(env_u64("RAVEN_SOAK_TIMEOUT_SECS", 60));

    let parallel_src = read_example("concurrency_parallel.rv");
    let cases: [(&str, &str, &str); 2] = [
        ("concurrency_parallel", &parallel_src, "48000"),
        ("spin_nonalloc", SPIN_SOURCE, "5"),
    ];

    let mut failures = Vec::new();
    for (name, source, expected) in cases {
        let prog = match build_program(source, &runtime) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("{name}: build failed: {e}"));
                continue;
            }
        };
        for run in 0..runs {
            match run_program(&prog.binary, &threshold, timeout) {
                Ok(out) if out == expected => {}
                Ok(out) => {
                    failures.push(format!("{name} run {run}: got {out:?}, want {expected:?}"));
                }
                Err(e) => failures.push(format!("{name} run {run}: {e}")),
            }
        }
    }

    assert!(
        failures.is_empty(),
        "concurrency soak found {} failure(s) over {runs} runs at threshold {threshold}:\n{}",
        failures.len(),
        failures.join("\n"),
    );
}

/// Run `binary` once and capture its trimmed stdout, killing it (and reporting a
/// hang) if it does not finish within `timeout`. The child's stdout is drained
/// on a helper thread so a full pipe cannot wedge it.
fn run_program(binary: &Path, threshold: &str, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(binary)
        .env("RAVEN_GC_THRESHOLD", threshold)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn: {e}"))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {}
            Err(e) => return Err(format!("wait: {e}")),
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let out = reader.join().unwrap_or_default();
    match status {
        None => Err(format!("HANG (no exit within {timeout:?})")),
        Some(st) if st.success() => Ok(out.trim().to_string()),
        Some(st) => Err(format!("exit {:?}, stdout {:?}", st.code(), out.trim())),
    }
}

fn read_example(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/v2")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

struct CompiledProgram {
    binary: PathBuf,
    tmp: PathBuf,
}

impl Drop for CompiledProgram {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

fn build_program(source: &str, runtime: &RuntimeStaticLib) -> Result<CompiledProgram, String> {
    let object_bytes = build_object(source)?;
    let tmp = workdir();
    let object_path = tmp.join("gen.o");
    std::fs::write(&object_path, &object_bytes).expect("write object");
    let binary = tmp.join(if cfg!(windows) { "gen.exe" } else { "gen" });
    if let Err(e) = linker::link(&object_path, runtime, &binary) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("link: {e}"));
    }
    Ok(CompiledProgram { binary, tmp })
}

const COMPILER_STACK_SIZE: usize = 512 * 1024 * 1024;

fn build_object(source: &str) -> Result<Vec<u8>, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(COMPILER_STACK_SIZE)
        .spawn(move || build_object_inner(&source))
        .expect("spawn compile worker")
        .join()
        .expect("compile worker panicked")
}

fn build_object_inner(source: &str) -> Result<Vec<u8>, String> {
    let path = Path::new("generated.rv");
    let tokens = Lexer::new(source.to_string(), path.to_path_buf())
        .tokenize()
        .map_err(|e| format!("lex: {e}"))?;
    let tokens = raven::macros::expand_tokens(&tokens).map_err(|e| format!("macro: {e}"))?;
    let file = parse(&tokens).map_err(|e| format!("parse: {e}"))?;
    let file = expand_with_stdlib(&file).map_err(|e| format!("stdlib: {e}"))?;
    let mut loader = FsLoader;
    let resolved = resolve_file(&file, &mut loader).map_err(|e| format!("resolve: {e}"))?;
    let typed = check_file(&resolved).map_err(|e| format!("tycheck: {e}"))?;
    let hir = lower_file(&typed).map_err(|e| format!("hir: {e}"))?;
    let mir = lower_program(&hir).map_err(|e| format!("mir: {e}"))?;
    codegen::compile_program(&mir).map_err(|e: CodegenError| format!("codegen: {e}"))
}

fn workdir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    p.push(format!("raven-soak-{pid}-{stamp}-{seq}"));
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

fn supported_runtime() -> Option<RuntimeStaticLib> {
    if !linker::linker_available() {
        eprintln!("concurrency_soak: skipping, no linker available for the host.");
        return None;
    }
    locate_runtime().or_else(|| {
        eprintln!(
            "concurrency_soak: skipping, raven_runtime staticlib not built. \
             Run `cargo build -p raven-runtime`."
        );
        None
    })
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
