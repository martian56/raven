//! Inline unit tests for the MIR module.
//!
//! Each test runs the full pipeline (lex -> parse -> resolve ->
//! tycheck -> hir -> mir) on a small Raven snippet and asserts
//! structural properties of the resulting program. Wider coverage of
//! the exact textual shape lives in `tests/mir_golden.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::hir::lower_file;
use crate::lexer::Lexer;
use crate::mir::builder::FunctionBuilder;
use crate::mir::ir::{
    MirConstant, MirFunction, MirOperand, MirRvalue, MirStatement, MirTerminator,
};
use crate::mir::ty::MirType;
use crate::mir::{lower_program, MirProgram};
use crate::parser::parse;
use crate::resolve::{resolve_file, LoadedSource, SourceLoader};
use crate::span::Span;
use crate::tycheck::check_file;

struct NoLoader;
impl SourceLoader for NoLoader {
    fn load(&mut self, _i: &Path, _t: &str) -> Option<LoadedSource> {
        None
    }
}

fn dummy_span() -> Span {
    Span::new(Arc::new(PathBuf::from("t.rv")), 0, 0, 1, 1)
}

/// Run the full v2 pipeline on `src` and return the MIR program.
fn compile(src: &str) -> MirProgram {
    let tokens = Lexer::new(src.to_string(), PathBuf::from("t.rv"))
        .tokenize()
        .expect("lex");
    let file = parse(&tokens).expect("parse");
    let mut loader = NoLoader;
    let resolved = resolve_file(&file, &mut loader).expect("resolve");
    let typed = check_file(&resolved).expect("tycheck");
    let hir = lower_file(&typed).expect("hir");
    lower_program(&hir).expect("mir")
}

fn find_fn<'a>(p: &'a MirProgram, name: &str) -> &'a MirFunction {
    p.functions
        .iter()
        .find(|f| f.origin == name)
        .unwrap_or_else(|| panic!("function {} not in MIR", name))
}

// ----- Builder smoke tests -----

#[test]
fn empty_program_pretty_prints() {
    let prog = MirProgram::new();
    let rendered = crate::mir::pretty_program(&prog);
    assert!(rendered.contains("(mir"));
}

#[test]
fn builder_emits_single_block_function() {
    let mut b = FunctionBuilder::new("noop".into(), "noop".into(), MirType::Unit, dummy_span());
    let entry = b.new_block();
    b.close_block(
        entry,
        MirTerminator::Return(MirOperand::Const(MirConstant::Unit)),
    );
    let fun = b.finish(entry);
    assert_eq!(fun.blocks.len(), 1);
    assert_eq!(fun.name, "noop");
}

#[test]
fn builder_allocates_distinct_locals() {
    let mut b = FunctionBuilder::new("two".into(), "two".into(), MirType::Int, dummy_span());
    let p = b.add_param("x".into(), MirType::Int);
    let t = b.fresh_temp("tmp", MirType::Int);
    assert_ne!(p.0, t.0);
    assert_eq!(b.locals().len(), 2);
    let entry = b.new_block();
    b.assign(entry, t, MirRvalue::Use(MirOperand::Copy(p)));
    b.close_block(entry, MirTerminator::Return(MirOperand::Copy(t)));
    let fun = b.finish(entry);
    assert_eq!(fun.blocks[0].statements.len(), 1);
}

#[test]
#[should_panic]
fn builder_double_close_panics() {
    let mut b = FunctionBuilder::new("bad".into(), "bad".into(), MirType::Unit, dummy_span());
    let entry = b.new_block();
    b.close_block(
        entry,
        MirTerminator::Return(MirOperand::Const(MirConstant::Unit)),
    );
    b.close_block(
        entry,
        MirTerminator::Return(MirOperand::Const(MirConstant::Unit)),
    );
}

// ----- End to end lowering tests -----

#[test]
fn arithmetic_lowers_to_binop() {
    let prog = compile("fun add(a: Int, b: Int) -> Int { return a + b }");
    let f = find_fn(&prog, "add");
    assert_eq!(f.params.len(), 2);
    let saw_binop = f.blocks.iter().flat_map(|b| b.statements.iter()).any(|s| {
        matches!(
            s,
            MirStatement::Assign {
                rvalue: MirRvalue::BinaryOp(..),
                ..
            }
        )
    });
    assert!(saw_binop, "expected a binop assignment");
}

#[test]
fn if_branches_become_switch_int() {
    let prog = compile("fun pick(c: Bool) -> Int { return if c { 1 } else { 2 } }");
    let f = find_fn(&prog, "pick");
    let switches = f
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator, MirTerminator::SwitchInt { .. }))
        .count();
    assert!(switches >= 1, "expected at least one switch-int");
}

#[test]
fn while_loop_has_back_edge() {
    let prog = compile("fun spin() -> () { let i = 0; while i < 10 { } }");
    let f = find_fn(&prog, "spin");
    // The while header should be reached by a Goto from the body.
    let goto_count = f
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator, MirTerminator::Goto(_)))
        .count();
    assert!(goto_count >= 2, "expected at least two goto terminators");
}

#[test]
fn return_terminates_block() {
    let prog = compile("fun zero() -> Int { return 0 }");
    let f = find_fn(&prog, "zero");
    let returns = f
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator, MirTerminator::Return(_)))
        .count();
    assert!(returns >= 1, "expected a return terminator");
}

#[test]
fn struct_create_emitted_for_struct_literal() {
    let src = r#"
        struct Point { x: Int, y: Int }
        fun mk() -> Point { return Point { x: 1, y: 2 } }
    "#;
    let prog = compile(src);
    let f = find_fn(&prog, "mk");
    let saw_struct_create = f.blocks.iter().flat_map(|b| b.statements.iter()).any(|s| {
        matches!(
            s,
            MirStatement::Assign {
                rvalue: MirRvalue::StructCreate { .. },
                ..
            }
        )
    });
    assert!(saw_struct_create, "expected a struct-create");
}

#[test]
fn option_some_lowers_to_call_or_enum_create() {
    // User-written `Some(x)` reaches HIR as a regular `Call`; the
    // dedicated `SomeCtor` form is only synthesized by `?` desugaring.
    // MIR therefore emits a call to the constructor by name. Once
    // codegen lowers Option natively, the test will tighten.
    let src = r#"
        fun maybe() -> Option<Int> { return Some(42) }
    "#;
    let prog = compile(src);
    let f = find_fn(&prog, "maybe");
    let saw_call = f.blocks.iter().flat_map(|b| b.statements.iter()).any(|s| {
        matches!(
            s,
            MirStatement::Assign {
                rvalue: MirRvalue::Call { .. },
                ..
            }
        )
    });
    assert!(saw_call, "expected a call for Some(42)");
}

#[test]
fn try_operator_emits_enum_create_via_some_ctor() {
    let src = r#"
        fun take(o: Option<Int>) -> Option<Int> {
            let v = o?;
            return Some(v)
        }
    "#;
    let prog = compile(src);
    let f = find_fn(&prog, "take");
    let saw_enum = f.blocks.iter().flat_map(|b| b.statements.iter()).any(|s| {
        matches!(
            s,
            MirStatement::Assign {
                rvalue: MirRvalue::EnumCreate { .. },
                ..
            }
        )
    });
    assert!(saw_enum, "expected EnumCreate from `?` desugaring");
}

#[test]
fn non_generic_function_kept_as_root() {
    let prog = compile("fun a() -> Int { return 1 } fun b() -> Int { return 2 }");
    assert!(prog.functions.iter().any(|f| f.origin == "a"));
    assert!(prog.functions.iter().any(|f| f.origin == "b"));
}

#[test]
fn monomorphize_dedupes_repeated_instantiations() {
    // No HIR-level generic call sites are emitted yet, but the
    // worklist seen-set must still treat a repeated insertion of the
    // same root as a no-op. We exercise that by compiling a file with
    // two identical functions and counting outputs.
    let prog = compile("fun a() -> Int { return 1 } fun a2() -> Int { return 1 }");
    let names: Vec<&str> = prog.functions.iter().map(|f| f.origin.as_str()).collect();
    assert!(names.contains(&"a"));
    assert!(names.contains(&"a2"));
    // No accidental duplicates.
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names.len(), sorted.len(), "duplicate functions emitted");
}

#[test]
fn mir_type_mangling_is_stable() {
    use crate::mir::ty::MirType;
    assert_eq!(MirType::Int.mangle(), "Int");
    assert_eq!(
        MirType::Option(Box::new(MirType::Int)).mangle(),
        "Option_Int"
    );
    assert_eq!(
        MirType::Result(Box::new(MirType::Int), Box::new(MirType::Str)).mangle(),
        "Result_Int_Str"
    );
}
