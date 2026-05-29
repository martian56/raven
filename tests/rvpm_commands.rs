//! End to end tests for `rvpm add`, `rvpm install`, and `rvpm update`.
//!
//! These drive the actual `rvpm` binary in a temp project directory with
//! `RVPM_CACHE_DIR` pointed at a pre-seeded cache, so no test contacts the
//! network. The env override is process-global, so the CLI tests that set
//! it run their steps sequentially within one test function and use a
//! cache root unique to that test. Library-level coverage of each command
//! lives in `raven::ops` unit tests.

use std::path::{Path, PathBuf};
use std::process::Command;

use raven::lock::{LockFile, LOCK_FILE_NAME};
use raven::manifest::Manifest;
use raven::pkg;
use raven::resolve::GithubPath;

/// Seed a cache entry under `cache_root` with the given files.
fn seed(cache_root: &Path, source: &str, version: &str, files: &[(&str, &str)]) {
    let gh = GithubPath::parse(source).expect("github path");
    let dir = pkg::cache_dir_in(cache_root, &gh.host, &gh.user, &gh.repo, version);
    std::fs::create_dir_all(&dir).expect("create cache dir");
    for (rel, contents) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, contents).expect("write seed file");
    }
}

fn rvpm(project: &Path, cache_root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rvpm"))
        .args(args)
        .current_dir(project)
        .env("RVPM_CACHE_DIR", cache_root)
        .output()
        .expect("run rvpm")
}

#[test]
fn add_install_update_via_cli() {
    let root = workdir();
    let project = root.join("project");
    let cache = root.join("cache");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("rv.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n# user comment\n",
    )
    .unwrap();

    seed(
        &cache,
        "github.com/acme/foo",
        "v1.0.0",
        &[(
            "rv.toml",
            "[package]\nname = \"foo\"\nversion = \"1.0.0\"\n",
        )],
    );
    seed(
        &cache,
        "github.com/acme/foo",
        "v1.1.0",
        &[(
            "rv.toml",
            "[package]\nname = \"foo\"\nversion = \"1.1.0\"\n",
        )],
    );

    // add: writes the dependency and a lock.
    let out = rvpm(&project, &cache, &["add", "github.com/acme/foo@v1.0.0"]);
    assert!(
        out.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let manifest_text = std::fs::read_to_string(project.join("rv.toml")).unwrap();
    assert!(manifest_text.contains("github.com/acme/foo"));
    assert!(manifest_text.contains("user comment"), "comment preserved");
    let m = Manifest::from_toml_str(&manifest_text).expect("manifest re-parses");
    assert!(m
        .dependencies
        .iter()
        .any(|d| d.path == "github.com/acme/foo"));
    let lock = LockFile::load(project.join(LOCK_FILE_NAME)).expect("lock present");
    assert_eq!(lock.packages[0].version, "v1.0.0");

    // install: validates the existing lock.
    let out = rvpm(&project, &cache, &["install"]);
    assert!(
        out.status.success(),
        "install failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Validated"), "install stdout: {}", stdout);

    // install detects a tampered cache and aborts non-zero.
    let gh = GithubPath::parse("github.com/acme/foo").unwrap();
    let dir = pkg::cache_dir_in(&cache, &gh.host, &gh.user, &gh.repo, "v1.0.0");
    let f = dir.join("rv.toml");
    let mut contents = std::fs::read_to_string(&f).unwrap();
    contents.push_str("\n# tampered\n");
    std::fs::write(&f, contents).unwrap();
    let out = rvpm(&project, &cache, &["install"]);
    assert!(!out.status.success(), "tampered install should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("hash mismatch"), "stderr: {}", stderr);

    // update: bump the ref in rv.toml, then update the package.
    std::fs::write(
        project.join("rv.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"github.com/acme/foo\" = \"v1.1.0\"\n",
    )
    .unwrap();
    let out = rvpm(&project, &cache, &["update", "github.com/acme/foo"]);
    assert!(
        out.status.success(),
        "update failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lock = LockFile::load(project.join(LOCK_FILE_NAME)).expect("lock present");
    let foo = lock
        .packages
        .iter()
        .find(|p| p.source == "github.com/acme/foo")
        .unwrap();
    assert_eq!(foo.version, "v1.1.0");

    cleanup(&root);
}

#[test]
fn add_without_version_records_placeholder() {
    let root = workdir();
    let project = root.join("project");
    let cache = root.join("cache");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("rv.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    // No cache entry seeded for the placeholder ref, so resolution fails
    // and the command exits non-zero, but the manifest edit must persist.
    let out = rvpm(&project, &cache, &["add", "github.com/acme/foo"]);
    assert!(!out.status.success(), "placeholder add cannot resolve");
    let manifest_text = std::fs::read_to_string(project.join("rv.toml")).unwrap();
    assert!(manifest_text.contains("github.com/acme/foo"));
    assert!(manifest_text.contains("latest"), "placeholder recorded");
    cleanup(&root);
}

#[test]
fn add_bad_path_fails() {
    let root = workdir();
    let project = root.join("project");
    let cache = root.join("cache");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("rv.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let out = rvpm(&project, &cache, &["add", "gitlab.com/a/b@v1.0.0"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("github.com"), "stderr: {}", stderr);
    cleanup(&root);
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
    p.push(format!("rvpm-cmd-{}-{}-{}", std::process::id(), stamp, seq));
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

fn cleanup(p: &Path) {
    let _ = std::fs::remove_dir_all(p);
}
