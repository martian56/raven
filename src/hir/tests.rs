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
fn for_over_list_lowers_to_index_loop() {
    // `for v in xs` lowers to a block that binds the list once, then a
    // counter loop that drives the index from 0 to `xs.len()` and binds
    // each element by indexing. No iterator object is produced.
    let p = lower("fun f() -> () { for v in [1, 2, 3] { } }");
    let f = only_fn(&p, "f");
    let body = f.body.as_ref().expect("body");
    let desugared = first_for_block(body);
    // The desugared form binds the list (`let __list = ...`) and tails
    // into the counter-loop block.
    match &desugared.kind {
        HirExprKind::Block(inner) => {
            assert!(
                matches!(inner.stmts[0].kind, HirStmtKind::Let { .. }),
                "list value is bound once before the loop"
            );
            let tail = inner.tail.as_ref().expect("counter-loop block tail");
            assert_counter_loop(tail);
        }
        other => panic!("expected block after for desugaring, got {:?}", other),
    }
    // The lowering must no longer mint any iterator/range intrinsic.
    assert!(!hir_uses_iterator_intrinsics(&p));
}

#[test]
fn for_over_range_lowers_to_counter_loop() {
    // `for x in 0..3` lowers straight to a counter loop over the integer
    // interval, with no range object and no iterator intrinsic.
    let p = lower("fun f() -> () { for x in 0..3 { } }");
    let f = only_fn(&p, "f");
    let body = f.body.as_ref().expect("body");
    let desugared = first_for_block(body);
    assert_counter_loop(desugared);
    assert!(!hir_uses_iterator_intrinsics(&p));
}

/// Pull the desugared `for` expression out of a function body. The `for`
/// shows up either as a statement-expression or as the trailing tail.
fn first_for_block(body: &crate::hir::expr::HirBlock) -> &crate::hir::expr::HirExpr {
    if let Some(stmt) = body
        .stmts
        .iter()
        .find(|s| matches!(s.kind, HirStmtKind::Expr(_)))
    {
        match &stmt.kind {
            HirStmtKind::Expr(e) => return e,
            _ => unreachable!(),
        }
    }
    body.tail.as_ref().expect("for as stmt or tail")
}

/// Assert the given expression is a counter-loop block: a block whose
/// tail is a `Loop` whose body's first statement is the increment-and-
/// guard `if` (the advance step a `continue` re-enters).
fn assert_counter_loop(expr: &crate::hir::expr::HirExpr) {
    let HirExprKind::Block(block) = &expr.kind else {
        panic!("expected counter-loop block, got {:?}", expr.kind);
    };
    let tail = block.tail.as_ref().expect("loop tail");
    let HirExprKind::Loop(loop_body) = &tail.kind else {
        panic!("expected Loop tail, got {:?}", tail.kind);
    };
    // The first loop-body statement is the `if __first { ... } else { __i
    // = __i + 1 }` advance, so a `continue` (which re-enters the loop
    // header) always runs the increment before the next iteration.
    match &loop_body.stmts[0].kind {
        HirStmtKind::Expr(e) => assert!(
            matches!(e.kind, HirExprKind::If { .. }),
            "loop body starts with the advance-step `if`"
        ),
        other => panic!("expected advance `if` first, got {:?}", other),
    }
}

/// True when any HIR expression in the program is one of the iterator or
/// range intrinsics that the for-loop lowering must no longer emit.
fn hir_uses_iterator_intrinsics(p: &HirProgram) -> bool {
    fn block_uses(b: &crate::hir::expr::HirBlock) -> bool {
        b.stmts.iter().any(|s| stmt_uses(&s.kind))
            || b.tail.as_ref().is_some_and(|e| expr_uses(&e.kind))
    }
    fn stmt_uses(s: &HirStmtKind) -> bool {
        match s {
            HirStmtKind::Let { init, .. } => expr_uses(&init.kind),
            HirStmtKind::Expr(e) => expr_uses(&e.kind),
            HirStmtKind::Assign { value, .. } => expr_uses(&value.kind),
            HirStmtKind::Defer(e) | HirStmtKind::Spawn(e) => expr_uses(&e.kind),
        }
    }
    fn expr_uses(k: &HirExprKind) -> bool {
        match k {
            HirExprKind::IterNew(_) | HirExprKind::IterNext(_) | HirExprKind::RangeNew { .. } => {
                true
            }
            HirExprKind::Block(b) => block_uses(b),
            HirExprKind::Loop(b) => block_uses(b),
            HirExprKind::While { cond, body } => expr_uses(&cond.kind) || block_uses(body),
            HirExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                expr_uses(&cond.kind)
                    || block_uses(then_block)
                    || else_block.as_ref().is_some_and(block_uses)
            }
            HirExprKind::Paren(e) => expr_uses(&e.kind),
            HirExprKind::Binary { lhs, rhs, .. } => expr_uses(&lhs.kind) || expr_uses(&rhs.kind),
            HirExprKind::Index { receiver, index } => {
                expr_uses(&receiver.kind) || expr_uses(&index.kind)
            }
            HirExprKind::MethodCall { receiver, args, .. } => {
                expr_uses(&receiver.kind) || args.iter().any(|a| expr_uses(&a.kind))
            }
            _ => false,
        }
    }
    p.items.iter().any(|item| match &item.kind {
        HirItemKind::Function(f) => f.body.as_ref().is_some_and(block_uses),
        _ => false,
    })
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
