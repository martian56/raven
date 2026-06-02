//! Golden snapshot tests for HIR -> MIR lowering and monomorphization.
//!
//! For every `tests/mir_corpus/*.rv` source file this test runs the
//! full pipeline (lex -> parse -> resolve -> tycheck -> hir -> mir)
//! and compares a textual dump of the resulting MIR program against
//! a committed `.rv.mir` baseline.
//!
//! Refresh baselines with `RAVEN_UPDATE_MIR_GOLDEN=1 cargo test
//! --test mir_golden`.

use std::fs;
use std::path::{Path, PathBuf};

use raven::hir::lower_file;
use raven::lexer::Lexer;
use raven::mir::{lower_program, pretty_program};
use raven::parser::parse;
use raven::resolve::{resolve_file, LoadedSource, SourceLoader};
use raven::tycheck::check_file;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("mir_corpus")
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
fn mir_golden() {
    let dir = corpus_dir();
    let update = std::env::var("RAVEN_UPDATE_MIR_GOLDEN").is_ok();

    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("corpus directory must exist")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rv"))
        .collect();
    entries.sort();

    assert!(!entries.is_empty(), "mir corpus is empty");

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
        let hir = match lower_file(&typed) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!(
                    "{}: hir error: {}",
                    src_path.display(),
                    e.display(&src)
                ));
                continue;
            }
        };
        let mir = match lower_program(&hir) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!(
                    "{}: mir error: {}",
                    src_path.display(),
                    e.display(&src)
                ));
                continue;
            }
        };
        let rendered = pretty_program(&mir);

        let baseline = src_path.with_extension("rv.mir");
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
                "missing baseline for {} (run with RAVEN_UPDATE_MIR_GOLDEN=1)",
                src_path.display()
            )),
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} golden MIR snapshot failure(s):\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}

fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n")
}
