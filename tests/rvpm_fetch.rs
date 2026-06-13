//! Network-free tests for the rvpm package cache and fetch.
//!
//! These drive the fetch against a LOCAL git repository created in a temp
//! directory, so no test contacts the network. git is required and is
//! present on dev and CI machines. Commits set a per-invocation identity
//! with `-c user.email`/`-c user.name` so they work on a clean CI machine
//! with no global git configuration.

use std::path::{Path, PathBuf};
use std::process::Command;

use raven::pkg::{self, PkgError};

/// Create a source git repository in `dir` with one file and tag it
/// `v1.0.0`. Returns its path, which `git clone` accepts as a source.
fn make_source_repo(dir: &Path) -> String {
    git(dir, &["init", "-q"]);
    git(dir, &["checkout", "-q", "-B", "main"]);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("rv.toml"),
        "[package]\nname = \"dep\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("lib.rv"),
        "fun answer() -> Int { 42 }\n",
    )
    .unwrap();
    git(dir, &["add", "."]);
    git(
        dir,
        &[
            "-c",
            "user.email=ci@example.com",
            "-c",
            "user.name=ci",
            "commit",
            "-q",
            "-m",
            "initial",
        ],
    );
    git(dir, &["tag", "v1.0.0"]);
    // A file path is a valid clone source. Pass it through git's own
    // path handling rather than building a file:// URL by hand.
    dir.to_string_lossy().into_owned()
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("run git");
    assert!(status.success(), "git {:?} failed", args);
}

#[test]
fn clone_from_lands_in_cache_and_drops_git() {
    let root = workdir();
    let src = root.join("src-repo");
    std::fs::create_dir_all(&src).unwrap();
    let url = make_source_repo(&src);

    let dest = root.join("cache").join("dep@v1.0.0");
    pkg::clone_from(&url, "v1.0.0", &dest).expect("clone into cache");

    assert!(
        dest.join("src").join("lib.rv").is_file(),
        "cloned working tree landed in the cache"
    );
    assert!(
        !dest.join(".git").exists(),
        ".git is removed to keep the cache lean"
    );

    cleanup(&root);
}

#[test]
fn missing_tag_reports_missing_ref() {
    let root = workdir();
    let src = root.join("src-repo");
    std::fs::create_dir_all(&src).unwrap();
    let url = make_source_repo(&src);

    let dest = root.join("cache").join("dep@v9.9.9");
    let err = pkg::clone_from(&url, "v9.9.9", &dest).unwrap_err();
    match err {
        PkgError::MissingRef { reference, .. } => assert_eq!(reference, "v9.9.9"),
        other => panic!("expected MissingRef, got {:?}", other),
    }

    cleanup(&root);
}

/// Exercises `fetch`, which derives the cache root from `RVPM_CACHE_DIR`.
/// The env override is process-global, so this single test owns it and
/// runs its steps sequentially. Other tests use `clone_from` directly and
/// never read the env.
#[test]
fn fetch_clones_then_hits_cache() {
    let root = workdir();
    let src = root.join("src-repo");
    std::fs::create_dir_all(&src).unwrap();
    let url = make_source_repo(&src);

    // fetch builds an https remote, covered by clone_from above. Here we
    // verify cache_dir layout and the cache-hit path under the env
    // override: seed the exact directory fetch would use, then assert a
    // second fetch is served from the cache without the remote.
    let cache_root = root.join("cache-root");
    std::env::set_var("RVPM_CACHE_DIR", &cache_root);

    let expected = pkg::cache_dir("github.com", "acme", "dep", "v1.0.0");
    assert!(expected.starts_with(&cache_root), "cache honors override");

    // Populate the cache entry by cloning the local repo into the exact
    // path fetch would use.
    pkg::clone_from(&url, "v1.0.0", &expected).expect("seed cache");

    // Remove the source repo. A real fetch would now have to hit the
    // network; a cache HIT must not, so this must still succeed.
    std::fs::remove_dir_all(&src).unwrap();

    let got = pkg::fetch("github.com", "acme", "dep", "v1.0.0").expect("cache hit");
    assert_eq!(got.dir, expected);
    assert!(got.cached, "a populated entry is served from the cache");
    assert!(got.dir.join("src").join("lib.rv").is_file());

    std::env::remove_var("RVPM_CACHE_DIR");
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
    p.push(format!(
        "rvpm-fetch-{}-{}-{}",
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
