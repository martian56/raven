//! Inline unit tests for the MIR module.

use std::path::PathBuf;
use std::sync::Arc;

use crate::mir::builder::FunctionBuilder;
use crate::mir::ir::{MirOperand, MirRvalue, MirTerminator};
use crate::mir::ty::MirType;
use crate::span::Span;

fn dummy_span() -> Span {
    Span::new(Arc::new(PathBuf::from("t.rv")), 0, 0, 1, 1)
}

#[test]
fn empty_program_pretty_prints() {
    let prog = crate::mir::MirProgram::new();
    let rendered = crate::mir::pretty_program(&prog);
    assert!(rendered.contains("(mir"));
}

#[test]
fn builder_emits_single_block_function() {
    let mut b = FunctionBuilder::new("noop".into(), "noop".into(), MirType::Unit, dummy_span());
    let entry = b.new_block();
    b.close_block(
        entry,
        MirTerminator::Return(MirOperand::Const(crate::mir::ir::MirConstant::Unit)),
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
        MirTerminator::Return(MirOperand::Const(crate::mir::ir::MirConstant::Unit)),
    );
    b.close_block(
        entry,
        MirTerminator::Return(MirOperand::Const(crate::mir::ir::MirConstant::Unit)),
    );
}
