//! End to end tests for `rvpm fmt` and `rvpm fmt --check`, driving the
//! actual binary on temp files.

use std::path::PathBuf;
use std::process::Command;

fn rvpm(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rvpm"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run rvpm")
}

fn workdir() -> PathBuf {
    let mut p = std::env::temp_dir();
    let unique = format!(
        "rvpm_fmt_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    p.push(unique);
    std::fs::create_dir_all(&p).unwrap();
    p
}

const MESSY: &str = "fun  main ( ) {\nlet   x=1\nreturn x\n}";
const CANONICAL: &str = "fun main() {\n    let x = 1\n    return x\n}\n";

#[test]
fn check_flags_unformatted_file_and_format_fixes_it() {
    let dir = workdir();
    let file = dir.join("messy.rv");
    std::fs::write(&file, MESSY).unwrap();

    // --check reports a change and exits non-zero, naming the file.
    let out = rvpm(&dir, &["fmt", "--check", "messy.rv"]);
    assert!(!out.status.success(), "check should fail on messy file");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("messy.rv"), "stderr should name the file");

    // fmt rewrites the file canonically.
    let out = rvpm(&dir, &["fmt", "messy.rv"]);
    assert!(out.status.success(), "fmt should succeed");
    let after = std::fs::read_to_string(&file).unwrap();
    assert_eq!(after, CANONICAL);

    // --check now passes.
    let out = rvpm(&dir, &["fmt", "--check", "messy.rv"]);
    assert!(out.status.success(), "check should pass after formatting");
}

#[test]
fn check_passes_on_canonical_file() {
    let dir = workdir();
    let file = dir.join("clean.rv");
    std::fs::write(&file, CANONICAL).unwrap();
    let out = rvpm(&dir, &["fmt", "--check", "clean.rv"]);
    assert!(out.status.success(), "canonical file should pass --check");
}

#[test]
fn fmt_with_no_paths_formats_src_dir() {
    let dir = workdir();
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let file = src.join("main.rv");
    std::fs::write(&file, MESSY).unwrap();

    let out = rvpm(&dir, &["fmt"]);
    assert!(out.status.success());
    let after = std::fs::read_to_string(&file).unwrap();
    assert_eq!(after, CANONICAL);
}

#[test]
fn parse_error_reports_and_exits_nonzero() {
    let dir = workdir();
    let file = dir.join("bad.rv");
    std::fs::write(&file, "fun (").unwrap();
    let out = rvpm(&dir, &["fmt", "bad.rv"]);
    assert!(
        !out.status.success(),
        "fmt should fail on unparseable input"
    );
}
