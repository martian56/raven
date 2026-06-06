//! Compile-and-run golden suite for the v2 examples.
//!
//! Walks every runnable example under `examples/v2/`, compiles it with the
//! driver pipeline, links it against the `raven-runtime` staticlib, runs the
//! binary, and diffs its stdout against a committed `<name>.rv.out` baseline.
//! Set `RAVEN_UPDATE_GOLDEN=1` to (re)write the baselines instead of asserting.
//!
//! An example is excluded by placing `// golden:skip` in its first few lines.
//! Skipped examples are non-deterministic or need external setup (env vars, a
//! loopback server) and are covered by the dedicated cases in
//! `codegen_smoke.rs` instead.
//!
//! Like `codegen_smoke.rs`, the suite gates on a supported host: it short
//! circuits with a diagnostic when no linker is present or the runtime
//! staticlib has not been built. On a configured host it links and runs the
//! programs for real, so the baselines are an end to end check.

use std::path::{Path, PathBuf};
use std::process::Command;

use raven::codegen::linker::{self, RuntimeStaticLib};
use raven::codegen::{self, CodegenError};
use raven::hir::lower_file;
use raven::lexer::Lexer;
use raven::mir::lower_program;
use raven::parser::parse_with_macros;
use raven::resolve::{expand_with_stdlib, resolve_file, FsLoader};
use raven::tycheck::check_file;

#[test]
fn v2_examples_match_golden_baselines() {
    let Some(runtime) = supported_runtime() else {
        return;
    };

    let update = std::env::var_os("RAVEN_UPDATE_GOLDEN").is_some();
    let mut failures: Vec<String> = Vec::new();
    let mut ran = 0usize;

    for example in collect_examples() {
        let source = std::fs::read_to_string(&example.source)
            .unwrap_or_else(|e| panic!("read {}: {}", example.source.display(), e));

        let object_bytes = match build_object(&source, &example.source) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{}: build failed: {}", example.label, e));
                continue;
            }
        };

        let tmp = workdir();
        let object_path = tmp.join(format!("{}.o", example.stem));
        std::fs::write(&object_path, &object_bytes).expect("write object");
        let binary = tmp.join(if cfg!(windows) {
            format!("{}.exe", example.stem)
        } else {
            example.stem.clone()
        });
        if let Err(e) = linker::link(&object_path, &runtime, &binary) {
            cleanup(&tmp);
            failures.push(format!("{}: link failed: {}", example.label, e));
            continue;
        }

        let output = Command::new(&binary)
            .output()
            .unwrap_or_else(|e| panic!("run {} binary: {}", example.label, e));
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        cleanup(&tmp);

        if !output.status.success() {
            failures.push(format!(
                "{}: binary exited non zero: status={:?} stderr={}",
                example.label, output.status, stderr
            ));
            continue;
        }

        ran += 1;

        // Normalize line endings before storing or comparing: a program that
        // prints through C `printf` emits CRLF on Windows (the CRT text mode),
        // and git normalizes the committed `.rv.out` baseline to LF, so the
        // raw bytes would spuriously differ across platforms.
        let normalize = |s: &str| s.replace("\r\n", "\n");
        let stdout = normalize(&stdout);

        if update {
            std::fs::write(&example.baseline, &stdout)
                .unwrap_or_else(|e| panic!("write baseline {}: {}", example.baseline.display(), e));
            continue;
        }

        let expected = match std::fs::read_to_string(&example.baseline) {
            Ok(s) => normalize(&s),
            Err(e) => {
                failures.push(format!(
                    "{}: missing baseline {} ({}); run RAVEN_UPDATE_GOLDEN=1 to create it",
                    example.label,
                    example.baseline.display(),
                    e
                ));
                continue;
            }
        };
        if stdout != expected {
            failures.push(format!(
                "{}: stdout did not match baseline\n  expected: {:?}\n  actual:   {:?}",
                example.label, expected, stdout
            ));
        }
    }

    if update {
        eprintln!("golden: refreshed baselines for {} example(s)", ran);
        return;
    }

    assert!(
        failures.is_empty(),
        "golden suite failures ({} ran):\n{}",
        ran,
        failures.join("\n")
    );
    assert!(ran > 0, "golden suite ran no examples");
}

/// One runnable example: its entry source, the stem used to name temporary
/// artifacts, the committed `.rv.out` baseline, and a label for diagnostics.
struct Example {
    source: PathBuf,
    stem: String,
    baseline: PathBuf,
    label: String,
}

/// Gather the runnable examples. Top-level `examples/v2/*.rv` files that are
/// not marked `// golden:skip` are entries, plus the multi-file program at
/// `multifile/main.rv` (its sibling `helper.rv` is merged at resolve time and
/// is not a standalone entry). Helper sources in subdirectories are never
/// walked as entries, so the `*.rv` walk stays flat.
fn collect_examples() -> Vec<Example> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("v2");
    let mut examples = Vec::new();

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read examples/v2")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "rv").unwrap_or(false))
        .collect();
    entries.sort();

    for source in entries {
        if is_skipped(&source) {
            continue;
        }
        let stem = source.file_stem().unwrap().to_string_lossy().into_owned();
        let baseline = source.with_extension("rv.out");
        examples.push(Example {
            label: stem.clone(),
            source,
            stem,
            baseline,
        });
    }

    let multifile = dir.join("multifile").join("main.rv");
    if multifile.is_file() && !is_skipped(&multifile) {
        examples.push(Example {
            source: multifile.clone(),
            stem: "multifile_main".to_string(),
            baseline: multifile.with_extension("rv.out"),
            label: "multifile/main".to_string(),
        });
    }

    examples
}

/// An example opts out of the golden walk with `// golden:skip` in its first
/// five lines, mirroring the v1 golden convention.
fn is_skipped(path: &Path) -> bool {
    let Ok(source) = std::fs::read_to_string(path) else {
        return false;
    };
    source
        .lines()
        .take(5)
        .any(|line| line.contains("golden:skip"))
}

/// Return the runtime staticlib when a linker and the staticlib are both
/// present, or skip with a diagnostic. Mirrors the gate in `codegen_smoke.rs`.
fn supported_runtime() -> Option<RuntimeStaticLib> {
    if !linker::linker_available() {
        eprintln!(
            "golden: skipping, no linker available for the host. \
             Install the MSVC C++ build tools on windows-msvc, a 64-bit \
             MinGW-w64 on windows-gnu, or a `cc` driver on Unix."
        );
        return None;
    }
    match locate_runtime() {
        Some(r) => Some(r),
        None => {
            eprintln!(
                "golden: skipping, raven_runtime staticlib not built. \
                 Run `cargo build -p raven-runtime` to produce it."
            );
            None
        }
    }
}

/// Stack for the compile worker thread. The `raven` binary compiles on a
/// large-stack worker (issue #172) because lowering recurses with source
/// nesting; the golden suite compiles in process, so it does the same to
/// stay clear of the default test-thread stack on every host.
const COMPILER_STACK_SIZE: usize = 512 * 1024 * 1024;

fn build_object(source: &str, path: &Path) -> Result<Vec<u8>, String> {
    let source = source.to_string();
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .stack_size(COMPILER_STACK_SIZE)
        .spawn(move || build_object_inner(&source, &path))
        .expect("spawn compile worker")
        .join()
        .expect("compile worker panicked")
}

fn build_object_inner(source: &str, path: &Path) -> Result<Vec<u8>, String> {
    let tokens = Lexer::new(source.to_string(), path.to_path_buf())
        .tokenize()
        .map_err(|e| format!("lex: {}", e))?;
    let macro_table =
        raven::macros::collect_macro_table(&tokens).map_err(|e| format!("macro: {}", e))?;
    let tokens = raven::macros::expand_tokens(&tokens).map_err(|e| format!("macro: {}", e))?;
    let file = parse_with_macros(&tokens, macro_table).map_err(|e| format!("parse: {}", e))?;
    let file = expand_with_stdlib(&file).map_err(|e| format!("stdlib: {}", e))?;
    let mut loader = FsLoader;
    let resolved = resolve_file(&file, &mut loader).map_err(|e| format!("resolve: {}", e))?;
    let typed = check_file(&resolved).map_err(|e| format!("tycheck: {}", e))?;
    let hir = lower_file(&typed).map_err(|e| format!("hir: {}", e))?;
    let mir = lower_program(&hir).map_err(|e| format!("mir: {}", e))?;
    codegen::compile_program(&mir).map_err(|e: CodegenError| format!("codegen: {}", e))
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
    p.push(format!("raven-golden-{}-{}-{}", pid, stamp, seq));
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
