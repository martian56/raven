//! End to end test for `rvpm init`.
//!
//! Runs the `rvpm` binary in a temp directory and checks that it
//! scaffolds a parseable `rv.toml` and a `src/main.rv`. When a linker and
//! the runtime staticlib are available, the generated program is also
//! compiled with the `raven` binary and run, asserting its output.

use std::path::{Path, PathBuf};
use std::process::Command;

use raven::codegen::linker;
use raven::manifest::Manifest;

#[test]
fn rvpm_init_scaffolds_parseable_project() {
    let dir = workdir();
    let status = Command::new(env!("CARGO_BIN_EXE_rvpm"))
        .arg("init")
        .arg("demo_pkg")
        .current_dir(&dir)
        .status()
        .expect("run rvpm init");
    assert!(status.success(), "rvpm init exited non zero");

    let manifest_path = dir.join("rv.toml");
    let main_path = dir.join("src").join("main.rv");
    assert!(manifest_path.is_file(), "rv.toml was not created");
    assert!(main_path.is_file(), "src/main.rv was not created");

    let manifest_text = std::fs::read_to_string(&manifest_path).unwrap();
    let m = Manifest::from_toml_str(&manifest_text).expect("generated manifest parses");
    assert_eq!(m.package.name, "demo_pkg");
    assert_eq!(m.package.version, "0.1.0");
    assert!(m.dependencies.is_empty());

    let main_src = std::fs::read_to_string(&main_path).unwrap();
    assert!(main_src.contains("hello from demo_pkg"));

    maybe_compile_and_run(&dir, &main_path);

    cleanup(&dir);
}

#[test]
fn rvpm_init_refuses_existing_manifest() {
    let dir = workdir();
    std::fs::write(
        dir.join("rv.toml"),
        "[package]\nname=\"a\"\nversion=\"0.1.0\"\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rvpm"))
        .arg("init")
        .current_dir(&dir)
        .output()
        .expect("run rvpm init");
    assert!(
        !output.status.success(),
        "init should fail on existing rv.toml"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already exists"), "stderr: {}", stderr);
    cleanup(&dir);
}

#[test]
fn rvpm_unknown_subcommand_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_rvpm"))
        .arg("frobnicate")
        .output()
        .expect("run rvpm");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown subcommand"), "stderr: {}", stderr);
}

/// Compile and run the generated program when the host can link Raven
/// binaries; otherwise skip the compile step with a diagnostic, matching
/// the codegen smoke tests.
fn maybe_compile_and_run(dir: &Path, main_path: &Path) {
    if !linker::linker_available() || !runtime_built() {
        eprintln!("rvpm_init: skipping compile step, no linker or runtime staticlib available.");
        return;
    }
    let exe = dir.join(if cfg!(windows) { "demo.exe" } else { "demo" });
    let status = Command::new(env!("CARGO_BIN_EXE_raven"))
        .arg("build")
        .arg(main_path)
        .arg("-o")
        .arg(&exe)
        .status()
        .expect("run raven build");
    assert!(status.success(), "raven build failed for generated main.rv");

    let output = Command::new(&exe).output().expect("run generated binary");
    assert!(output.status.success(), "generated binary exited non zero");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hello from demo_pkg"),
        "unexpected stdout: {:?}",
        stdout
    );
}

fn runtime_built() -> bool {
    if std::env::var("RAVEN_RUNTIME_LIB").is_ok() {
        return true;
    }
    let lib_name = if cfg!(windows) {
        "raven_runtime.lib"
    } else {
        "libraven_runtime.a"
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    ["target/debug", "target/release"]
        .iter()
        .any(|sub| root.join(sub).join(lib_name).is_file())
}

fn workdir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut p = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    p.push(format!(
        "rvpm-init-{}-{}-{}",
        std::process::id(),
        stamp,
        seq
    ));
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

fn cleanup(p: &Path) {
    let _ = std::fs::remove_dir_all(p);
}
