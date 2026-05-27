//! Golden snapshot tests for the type checker.
//!
//! Walks every `tests/tycheck_corpus/*.rv`, lexes and parses the
//! source, runs the resolver and type checker against a no op
//! `SourceLoader`, and diffs a textual dump of every expression's
//! inferred type against a committed `.rv.types` baseline.
//!
//! Refresh baselines with `RAVEN_UPDATE_TYCHECK_GOLDEN=1 cargo test
//! --test tycheck_golden`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use raven::lexer::Lexer;
use raven::parser::parse;
use raven::resolve::{resolve_file, LoadedSource, SourceLoader};
use raven::tycheck::{check_file, TypeMap};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("tycheck_corpus")
}

/// Loader that never finds anything; corpus tests never use local
/// imports.
struct NoLoader;
impl SourceLoader for NoLoader {
    fn load(&mut self, _i: &Path, _t: &str) -> Option<LoadedSource> {
        None
    }
}

#[test]
fn tycheck_golden() {
    let dir = corpus_dir();
    let update = std::env::var("RAVEN_UPDATE_TYCHECK_GOLDEN").is_ok();

    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("corpus directory must exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rv"))
        .collect();
    entries.sort();

    assert!(!entries.is_empty(), "tycheck corpus is empty");

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
        let mut loader = NoLoader;
        let resolved = match resolve_file(&file, &mut loader) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!(
                    "{}: resolve error: {}",
                    src_path.display(),
                    e.display(&src)
                ));
                continue;
            }
        };
        let typed = match check_file(&resolved) {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!(
                    "{}: type error: {}",
                    src_path.display(),
                    e.display(&src)
                ));
                continue;
            }
        };
        let rendered = render_types(&typed.types, &src);

        let baseline = src_path.with_extension("rv.types");
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
                "missing baseline for {} (run with RAVEN_UPDATE_TYCHECK_GOLDEN=1)",
                src_path.display()
            )),
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} golden tycheck snapshot failure(s):\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}

/// Render the type map as a stable, human readable dump:
///
/// ```text
/// (types
///   0..3 "1+2" -> Int
///   ...
/// )
/// ```
fn render_types(map: &TypeMap, src: &str) -> String {
    let mut out = String::new();
    writeln!(&mut out, "(types").unwrap();
    for (key, ty) in map.sorted_iter() {
        let text = src
            .get(key.start..key.end)
            .map(|s| s.to_string())
            .unwrap_or_else(|| String::from("<oob>"));
        writeln!(
            &mut out,
            "  {}..{} \"{}\" -> {}",
            key.start,
            key.end,
            escape(&text),
            ty
        )
        .unwrap();
    }
    writeln!(&mut out, ")").unwrap();
    out
}

fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out
}

fn normalize(s: &str) -> String {
    // Normalize line endings so Windows CRLF baselines round trip.
    s.replace("\r\n", "\n")
}
