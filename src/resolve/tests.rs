//! Inline smoke test for the resolver. Real unit tests land in the
//! next commit.

#![cfg(test)]

use std::path::{Path, PathBuf};

use crate::lexer::Lexer;
use crate::parser::parse;

use super::imports::{LoadedSource, SourceLoader};
use super::resolve_file;

struct NoLoader;
impl SourceLoader for NoLoader {
    fn load(&mut self, _importing: &Path, _target: &str) -> Option<LoadedSource> {
        None
    }
}

#[test]
fn smoke_resolves_arithmetic_function() {
    let src = "fun mix(a: Int, b: Int, c: Int) -> Int = a + b * c - (a / b) % c\n";
    let tokens = Lexer::new(src.to_string(), PathBuf::from("test.rv"))
        .tokenize()
        .unwrap();
    let file = parse(&tokens).unwrap();
    let mut loader = NoLoader;
    let resolved = resolve_file(&file, &mut loader).expect("resolve should succeed");
    // The body references `a`, `b`, `c` each twice; each should be
    // bound as a parameter.
    let param_uses = resolved
        .map
        .uses
        .values()
        .filter(|b| matches!(b, super::Binding::Param(_)))
        .count();
    assert_eq!(param_uses, 6);
}
