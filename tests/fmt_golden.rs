//! Golden, idempotency, and semantic-preservation tests for the formatter.
//!
//! For every `tests/fmt_corpus/*.rv` source file this test formats the
//! source and compares against a committed `.rv.fmt` baseline. It also
//! verifies, for every corpus file and a sample of bundled stdlib files,
//! that formatting is idempotent (`format(format(x)) == format(x)`) and
//! semantics-preserving (`parse(format(x))` yields the same AST as
//! `parse(x)`, ignoring spans).
//!
//! Refresh baselines with `RAVEN_UPDATE_FMT=1 cargo test --test
//! fmt_golden`.

use std::fs;
use std::path::{Path, PathBuf};

use raven::ast::pretty_file;
use raven::format::format_source;
use raven::lexer::Lexer;
use raven::parser::parse;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fmt_corpus")
}

fn ast_dump(src: &str) -> String {
    let tokens = Lexer::new(src, "<t>").tokenize().expect("lex");
    let file = parse(&tokens).expect("parse");
    pretty_file(&file)
}

#[test]
fn fmt_golden() {
    let dir = corpus_dir();
    let update = std::env::var("RAVEN_UPDATE_FMT").is_ok();

    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("corpus directory must exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rv"))
        .collect();
    entries.sort();

    assert!(!entries.is_empty(), "fmt corpus is empty");

    let mut failures: Vec<String> = Vec::new();

    for src_path in entries {
        let src = fs::read_to_string(&src_path).expect("read source");
        let formatted = match format_source(&src) {
            Ok(f) => f,
            Err(e) => {
                failures.push(format!("{}: format error: {}", src_path.display(), e));
                continue;
            }
        };

        // Idempotency.
        let twice = format_source(&formatted).expect("format twice");
        if twice != formatted {
            failures.push(format!("{}: not idempotent", src_path.display()));
        }

        // Semantic preservation.
        if ast_dump(&src) != ast_dump(&formatted) {
            failures.push(format!(
                "{}: formatting changed the AST",
                src_path.display()
            ));
        }

        let golden_path = src_path.with_extension("rv.fmt");
        if update {
            fs::write(&golden_path, &formatted).expect("write golden");
            continue;
        }
        let expected = match fs::read_to_string(&golden_path) {
            Ok(s) => s,
            Err(_) => {
                failures.push(format!(
                    "{}: missing baseline (run RAVEN_UPDATE_FMT=1)",
                    golden_path.display()
                ));
                continue;
            }
        };
        if expected != formatted {
            failures.push(format!(
                "{}: output differs from baseline\n{}",
                src_path.display(),
                diff(&expected, &formatted)
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "fmt golden failures:\n{}",
        failures.join("\n")
    );
}

/// The bundled stdlib must format idempotently and without changing its
/// AST. Whether it is already byte-for-byte canonical is reported but not
/// enforced here (reformatting the stdlib is a separate change).
#[test]
fn stdlib_idempotent_and_semantics_preserving() {
    let stdlib = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("stdlib")
        .join("std");
    let mut entries: Vec<PathBuf> = fs::read_dir(&stdlib)
        .expect("stdlib dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rv"))
        .collect();
    entries.sort();
    assert!(!entries.is_empty());

    let mut failures = Vec::new();
    let mut already_canonical = 0usize;
    for path in &entries {
        let src = fs::read_to_string(path).expect("read");
        let formatted = match format_source(&src) {
            Ok(f) => f,
            Err(e) => {
                failures.push(format!("{}: {}", path.display(), e));
                continue;
            }
        };
        let twice = format_source(&formatted).expect("twice");
        if twice != formatted {
            failures.push(format!("{}: not idempotent", path.display()));
        }
        if ast_dump(&src) != ast_dump(&formatted) {
            failures.push(format!("{}: AST changed", path.display()));
        }
        if src == formatted {
            already_canonical += 1;
        }
    }
    eprintln!(
        "stdlib: {}/{} files already canonical under formatter rules",
        already_canonical,
        entries.len()
    );
    assert!(
        failures.is_empty(),
        "stdlib failures:\n{}",
        failures.join("\n")
    );
}

fn diff(expected: &str, actual: &str) -> String {
    let mut out = String::new();
    let e: Vec<&str> = expected.lines().collect();
    let a: Vec<&str> = actual.lines().collect();
    let n = e.len().max(a.len());
    for i in 0..n {
        let el = e.get(i).copied().unwrap_or("");
        let al = a.get(i).copied().unwrap_or("");
        if el != al {
            out.push_str(&format!("-{}\n+{}\n", el, al));
        }
    }
    out
}
