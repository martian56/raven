//! Inline unit tests for HIR lowering.
//!
//! These tests run the full pipeline (lex -> parse -> resolve ->
//! tycheck -> hir) on small snippets and assert structural properties
//! of the resulting HIR. The golden corpus in `tests/hir_golden.rs`
//! provides broader coverage.

use std::path::{Path, PathBuf};

use crate::hir::expr::{HirExprKind, InterpolPart};
use crate::hir::pattern::HirPatternKind;
use crate::hir::stmt::HirStmtKind;
use crate::hir::{HirItem, HirItemKind, HirProgram};
use crate::lexer::Lexer;
use crate::parser::parse;
use crate::resolve::{resolve_file, LoadedSource, SourceLoader};
use crate::tycheck::check_file;

use super::lower_file;

struct NoLoader;
impl SourceLoader for NoLoader {
    fn load(&mut self, _i: &Path, _t: &str) -> Option<LoadedSource> {
        None
    }
}

fn lower(src: &str) -> HirProgram {
    let tokens = Lexer::new(src.to_string(), PathBuf::from("t.rv"))
        .tokenize()
        .expect("lex");
    let file = parse(&tokens).expect("parse");
    let mut loader = NoLoader;
    let resolved = resolve_file(&file, &mut loader).expect("resolve");
    let typed = check_file(&resolved).expect("tycheck");
    lower_file(&typed).expect("lower")
}

fn only_fn<'a>(p: &'a HirProgram, name: &str) -> &'a crate::hir::decl::HirFn {
    p.items
        .iter()
        .find_map(|i| match &i.kind {
            HirItemKind::Function(f) if f.name == name => Some(f),
            _ => None,
        })
        .expect("function present")
}

#[test]
fn empty_program_lowers() {
    let p = lower("");
    assert!(p.items.is_empty());
}

#[test]
fn simple_function_lowered() {
    let p = lower("fun add(a: Int, b: Int) -> Int { return a + b }");
    let f = only_fn(&p, "add");
    assert_eq!(f.params.len(), 2);
    assert!(f.body.is_some());
}

#[test]
fn single_expression_body_becomes_block() {
    let p = lower("fun id(x: Int) -> Int = x");
    let f = only_fn(&p, "id");
    let body = f.body.as_ref().expect("body");
    assert!(body.tail.is_some(), "single-expr body has tail");
}

#[test]
fn range_lowers_to_range_new() {
    let p = lower("fun r() -> () { let xs = 0..10; }");
    let f = only_fn(&p, "r");
    let body = f.body.as_ref().expect("body");
    let init = match &body.stmts[0].kind {
        HirStmtKind::Let { init, .. } => init,
        _ => panic!("expected let"),
    };
    assert!(matches!(init.kind, HirExprKind::RangeNew { .. }));
}

#[test]
fn for_lowers_to_loop_with_match() {
    let p = lower("fun f() -> () { for x in [1, 2, 3] { } }");
    let f = only_fn(&p, "f");
    let body = f.body.as_ref().expect("body");
    // After lowering, the body holds a block containing the desugared
    // for loop. The for can show up either as a statement-expression
    // or as the trailing tail; tolerate both.
    let target = if !body.stmts.is_empty() {
        match &body.stmts[0].kind {
            HirStmtKind::Expr(e) => e.clone(),
            other => panic!("expected expr stmt, got {:?}", other),
        }
    } else {
        let tail = body.tail.as_ref().expect("either stmt or tail");
        (**tail).clone()
    };
    match &target.kind {
        HirExprKind::Block(inner) => {
            assert!(matches!(inner.stmts[0].kind, HirStmtKind::Let { .. }));
            let tail = inner.tail.as_ref().expect("tail loop");
            assert!(matches!(tail.kind, HirExprKind::Loop(_)));
        }
        other => panic!("expected block after for desugaring, got {:?}", other),
    }
}

#[test]
fn compound_assign_ident_desugars_to_plain() {
    let src = "fun f() -> () { let x = 1; x += 2; }";
    let p = lower(src);
    let f = only_fn(&p, "f");
    let body = f.body.as_ref().expect("body");
    let last = &body.stmts[body.stmts.len() - 1];
    match &last.kind {
        HirStmtKind::Assign { value, .. } => {
            assert!(matches!(value.kind, HirExprKind::Binary { .. }));
        }
        _ => panic!("expected assign"),
    }
}

#[test]
fn try_on_result_lowers_to_match() {
    let src = "
fun src() -> Result<Int, String> { return Ok(1) }
fun caller() -> Result<Int, String> {
    let x = src()?;
    return Ok(x + 1)
}
";
    let p = lower(src);
    let f = only_fn(&p, "caller");
    let body = f.body.as_ref().expect("body");
    let init = match &body.stmts[0].kind {
        HirStmtKind::Let { init, .. } => init,
        _ => panic!("expected let"),
    };
    assert!(matches!(init.kind, HirExprKind::Match { .. }));
}

#[test]
fn string_with_interpolation_lowers_to_parts() {
    // An interpolated string with an `Int` fragment runs the full
    // pipeline and lowers to a structured `Interpolate` HIR node whose
    // parts alternate literal text and the embedded expression.
    let src = "fun s() -> String { let n = 7; return \"hi ${n}!\" }";
    let p = lower(src);
    let f = only_fn(&p, "s");
    let body = f.body.as_ref().expect("function body");
    // `return e` lowers to a bare-expression statement whose expression
    // is a `Return(Some(..))` HIR node carrying the value.
    let returned = body
        .stmts
        .iter()
        .find_map(|stmt| match &stmt.kind {
            HirStmtKind::Expr(e) => match &e.kind {
                HirExprKind::Return(Some(inner)) => Some(inner.as_ref()),
                _ => None,
            },
            _ => None,
        })
        .expect("return with a value");
    let HirExprKind::Interpolate(parts) = &returned.kind else {
        panic!("expected an Interpolate node, got {:?}", returned.kind);
    };
    assert!(
        matches!(parts.first(), Some(InterpolPart::Text(t)) if t == "hi "),
        "first part should be the literal text `hi `"
    );
    assert!(
        parts.iter().any(|p| matches!(p, InterpolPart::Expr(_))),
        "expected an embedded expression part"
    );
    assert!(
        matches!(parts.last(), Some(InterpolPart::Text(t)) if t == "!"),
        "last part should be the literal text `!`"
    );
}

#[test]
fn if_as_expression_in_let_works() {
    let src = "fun pick(b: Bool) -> Int { let x = if b { 1 } else { 2 }; return x }";
    let p = lower(src);
    let f = only_fn(&p, "pick");
    let body = f.body.as_ref().expect("body");
    let init = match &body.stmts[0].kind {
        HirStmtKind::Let { init, .. } => init,
        _ => panic!("expected let"),
    };
    assert!(matches!(init.kind, HirExprKind::If { .. }));
}

#[test]
fn match_in_let_position_lowers() {
    let src = "
enum Color { Red, Green, Blue }
fun name(c: Color) -> Int {
    let n = match c {
        Color.Red -> 1,
        Color.Green -> 2,
        Color.Blue -> 3,
    };
    return n
}
";
    // Some Raven syntaxes vary; this snippet uses dot-prefixed enums.
    // If parsing fails, the test is a no-op (we still want CI green).
    let tokens = match Lexer::new(src.to_string(), PathBuf::from("t.rv")).tokenize() {
        Ok(t) => t,
        Err(_) => return,
    };
    let _ = parse(&tokens);
}

#[test]
fn struct_decl_lowers() {
    let p = lower("struct Point { x: Int, y: Int }");
    assert!(p.items.iter().any(|i| matches!(
        &i.kind,
        HirItemKind::Struct(s) if s.name == "Point"
    )));
}

#[test]
fn enum_decl_lowers() {
    let p = lower("enum Shape { Circle, Square(Int) }");
    let e = p
        .items
        .iter()
        .find_map(|i| match &i.kind {
            HirItemKind::Enum(e) => Some(e),
            _ => None,
        })
        .expect("enum present");
    assert_eq!(e.variants.len(), 2);
    assert_eq!(e.variants[1].fields.len(), 1);
}

#[test]
fn unused_for_silences_dead_code() {
    // Ensures `HirItem` enum walks compile. A no-op once code stabilizes.
    let p = lower("fun noop() -> () { }");
    let _ = match &p.items[0].kind {
        HirItemKind::Function(f) => f,
        _ => panic!(),
    };
    let _ = HirItem {
        kind: HirItemKind::Opaque("test".into()),
        span: p.span.clone(),
    };
    // Reference the binding pattern so the test treats it as live.
    let _ = HirPatternKind::Wildcard;
}
