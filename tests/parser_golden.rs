//! Golden snapshot tests for the parser.
//!
//! Walks every `tests/parser_corpus/*.rv`, parses it, pretty prints
//! the AST, and compares against a committed `.rv.ast` baseline next
//! to the source. Failures print a unified diff.
//!
//! Set `RAVEN_UPDATE_PARSER_GOLDEN=1` to overwrite baselines after an
//! intentional change.

use std::fs;
use std::path::{Path, PathBuf};

use raven::ast::pretty_file;
use raven::lexer::Lexer;
use raven::parser::parse;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("parser_corpus")
}

#[test]
fn parser_golden() {
    let dir = corpus_dir();
    let update = std::env::var("RAVEN_UPDATE_PARSER_GOLDEN").is_ok();

    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("corpus directory must exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rv"))
        .collect();
    entries.sort();

    assert!(!entries.is_empty(), "parser corpus is empty");

    let mut failures: Vec<String> = Vec::new();

    for src_path in entries {
        let src = fs::read_to_string(&src_path).expect("read source");
        let tokens = match Lexer::new(src.clone(), src_path.clone()).tokenize() {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!(
                    "{}: lex error: {}",
                    src_path.display(),
                    e.display(&src)
                ));
                continue;
            }
        };
        let file = match parse(&tokens) {
            Ok(f) => f,
            Err(e) => {
                failures.push(format!(
                    "{}: parse error: {}",
                    src_path.display(),
                    e.display(&src)
                ));
                continue;
            }
        };
        let rendered = pretty_file(&file);

        let baseline = src_path.with_extension("rv.ast");
        if update {
            fs::write(&baseline, &rendered).expect("write baseline");
            continue;
        }

        match fs::read_to_string(&baseline) {
            Ok(expected) => {
                if normalize(&expected) != normalize(&rendered) {
                    failures.push(format!(
                        "snapshot mismatch for {}\n--- expected ---\n{}\n--- actual ---\n{}",
                        src_path.display(),
                        expected,
                        rendered
                    ));
                }
            }
            Err(_) => failures.push(format!(
                "missing baseline for {} (run with RAVEN_UPDATE_PARSER_GOLDEN=1)",
                src_path.display()
            )),
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} golden parser snapshot failure(s):\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}

/// Normalize line endings before comparing so a Windows checkout with
/// CRLF baselines does not differ from a Unix render. We strip `\r`
/// and trim trailing whitespace per line.
fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n")
        .lines()
        .map(|l| l.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
