//! End to end test for `rvpm build` and `rvpm run` with a GitHub
//! dependency resolved through the rvpm cache.
//!
//! The acceptance criterion for issue #85 is that a multi-file project
//! with one GitHub dependency compiles and runs. This is exercised with
//! NO network by pre-seeding a temporary cache (the established pattern):
//! a cached `github.com/acme/greet@v1.0.0` package exports a function, the
//! project imports it, and the produced binary's stdout is asserted.
//!
//! The test drives the real `rvpm` binary (`CARGO_BIN_EXE_rvpm`), which in
//! turn drives the compile pipeline and the linker, so it is gated on a
//! supported runtime the same way the codegen smoke tests are: it skips
//! (passing) when no linker or no `raven_runtime` staticlib is present.

use std::path::{Path, PathBuf};
use std::process::Command;

use raven::codegen::linker;

#[test]
fn project_with_github_dependency_builds_and_runs() {
    if !supported_runtime() {
        return;
    }

    let work = workdir();
    let cache = work.join("cache");
    let project = work.join("project");
    std::fs::create_dir_all(&project).expect("mkdir project");

    // Pre-seed the cache with github.com/acme/greet@v1.0.0. Its lib.rv
    // exports `shout`, built on std/string's `concat`.
    let pkg_dir = cache.join("github.com").join("acme").join("greet@v1.0.0");
    std::fs::create_dir_all(&pkg_dir).expect("mkdir pkg");
    std::fs::write(
        pkg_dir.join("rv.toml"),
        "[package]\nname = \"greet\"\nversion = \"0.1.0\"\n",
    )
    .expect("write pkg toml");
    std::fs::write(
        pkg_dir.join("lib.rv"),
        "import std/string\nfun shout(s: String) -> String { return s.concat(\"!\") }\n",
    )
    .expect("write pkg lib");

    // The project: rv.toml with the dependency, and src/main.rv importing it.
    std::fs::write(
        project.join("rv.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/greet\" = \"v1.0.0\"\n",
    )
    .expect("write project toml");
    let src = project.join("src");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::write(
        src.join("main.rv"),
        "import \"github.com/acme/greet/lib\" { shout }\nfun main() { print(shout(\"hi\")) }\n",
    )
    .expect("write main");

    // `rvpm build`: resolves the lock against the pre-seeded cache, then
    // compiles src/main.rv with the dependency merged in.
    let build = rvpm(&project, &cache, &["build".to_string()]);
    assert!(
        build.status.success(),
        "rvpm build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(
        project.join("rv.lock").is_file(),
        "rvpm build should have written rv.lock"
    );
    let binary = project
        .join("target")
        .join("raven-out")
        .join(if cfg!(windows) { "demo.exe" } else { "demo" });
    assert!(
        binary.is_file(),
        "expected built binary at {}",
        binary.display()
    );

    // `rvpm run`: builds then runs, forwarding the program's exit code and
    // stdout. The dependency's `shout("hi")` prints `hi!`.
    let run = rvpm(&project, &cache, &["run".to_string()]);
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    cleanup(&work);
    assert!(
        run.status.success(),
        "rvpm run exited non zero: stderr={}",
        stderr
    );
    assert_eq!(stdout, "hi!\n", "unexpected program stdout: {:?}", stdout);
}

/// A dependency that bundles a C source through `[ffi]` is compiled and linked
/// into the consuming program: `cmath` ships `c/quad.c`, exposes it via an
/// `extern "C"` wrapper, and the project calls it. This is the shape a SQLite
/// binding (bundled `sqlite3.c`) takes.
#[test]
fn project_with_dependency_bundling_c_builds_and_runs() {
    if !supported_runtime() {
        return;
    }

    let work = workdir();
    let cache = work.join("cache");
    let project = work.join("project");
    std::fs::create_dir_all(&project).expect("mkdir project");

    let pkg_dir = cache.join("github.com").join("acme").join("cmath@v1.0.0");
    std::fs::create_dir_all(pkg_dir.join("c")).expect("mkdir pkg/c");
    std::fs::write(
        pkg_dir.join("rv.toml"),
        "[package]\nname = \"cmath\"\nversion = \"0.1.0\"\n\n[ffi]\nsources = [\"c/quad.c\"]\n",
    )
    .expect("write pkg toml");
    std::fs::write(
        pkg_dir.join("c").join("quad.c"),
        "int quad(int x) { return x * 4; }\n",
    )
    .expect("write pkg c");
    std::fs::write(
        pkg_dir.join("lib.rv"),
        "extern \"C\" {\n    fun quad(x: CInt) -> CInt\n}\nfun times4(x: CInt) -> CInt { return quad(x) }\n",
    )
    .expect("write pkg lib");

    std::fs::write(
        project.join("rv.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/cmath\" = \"v1.0.0\"\n",
    )
    .expect("write project toml");
    let src = project.join("src");
    std::fs::create_dir_all(&src).expect("mkdir src");
    std::fs::write(
        src.join("main.rv"),
        "import \"github.com/acme/cmath/lib\" { times4 }\nfun main() { print(times4(5)) }\n",
    )
    .expect("write main");

    let run = rvpm(&project, &cache, &["run".to_string()]);
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    cleanup(&work);
    assert!(
        run.status.success(),
        "rvpm run failed: stdout={} stderr={}",
        stdout,
        stderr
    );
    // times4(5) calls the bundled C quad(5) = 20.
    assert_eq!(
        stdout.trim(),
        "20",
        "unexpected program stdout: {:?}",
        stdout
    );
}

/// `rvpm test` links the package's own `[ffi]` so a package can test its native
/// bindings. A library bundles `c/inc.c`, wraps it, and a `*_test.rv` calls the
/// wrapper.
#[test]
fn rvpm_test_links_package_ffi() {
    if !supported_runtime() {
        return;
    }

    let work = workdir();
    let cache = work.join("cache");
    let project = work.join("lib");
    std::fs::create_dir_all(project.join("c")).expect("mkdir lib/c");

    std::fs::write(
        project.join("rv.toml"),
        "[package]\nname = \"inc\"\nversion = \"0.1.0\"\n\n[ffi]\nsources = [\"c/inc.c\"]\n",
    )
    .expect("write toml");
    std::fs::write(
        project.join("c").join("inc.c"),
        "#include <stdint.h>\nint64_t c_inc(int64_t x) { return x + 1; }\n",
    )
    .expect("write c");
    std::fs::write(
        project.join("lib.rv"),
        "extern \"C\" {\n    fun c_inc(x: Int) -> Int\n}\nfun inc(x: Int) -> Int { return c_inc(x) }\n",
    )
    .expect("write lib");
    std::fs::write(
        project.join("lib_test.rv"),
        "import std/test { assert_eq_int }\nimport \"./lib\" { inc }\nfun test_inc() { assert_eq_int(inc(4), 5) }\n",
    )
    .expect("write test");

    let out = rvpm(&project, &cache, &["test".to_string()]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    cleanup(&work);
    assert!(
        out.status.success(),
        "rvpm test failed: stdout={} stderr={}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("ok   test_inc"),
        "expected the test to pass: {}",
        stdout
    );
}

/// Invoke the real `rvpm` binary in `project_dir` with an isolated cache.
fn rvpm(project_dir: &Path, cache: &Path, args: &[String]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rvpm"))
        .args(args)
        .current_dir(project_dir)
        .env("RVPM_CACHE_DIR", cache)
        .output()
        .expect("run rvpm")
}

/// True when a linker and the runtime staticlib are both present. Mirrors
/// the codegen smoke gate so this test skips cleanly on hosts that cannot
/// link a native binary.
fn supported_runtime() -> bool {
    if !linker::linker_available() {
        eprintln!("rvpm_build: skipping, no linker available for the host.");
        return false;
    }
    if locate_runtime().is_none() {
        eprintln!("rvpm_build: skipping, raven_runtime staticlib not built.");
        return false;
    }
    true
}

fn locate_runtime() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RAVEN_RUNTIME_LIB") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
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
            return Some(p);
        }
    }
    None
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
    p.push(format!("rvpm-build-{}-{}-{}", pid, stamp, seq));
    std::fs::create_dir_all(&p).expect("create workdir");
    p
}

fn cleanup(p: &Path) {
    let _ = std::fs::remove_dir_all(p);
}
