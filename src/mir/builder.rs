//! CFG builder used by the HIR -> MIR lowering pass.
//!
//! The builder owns the in-progress [`MirFunction`] and offers a small
//! API: allocate locals, allocate basic blocks, emit statements into
//! the current block, and close blocks with terminators. Callers wire
//! the resulting CFG together by handing block ids back and forth.

use crate::span::Span;

use super::ir::{
    MirBlock, MirBlockId, MirFunction, MirLocal, MirLocalDecl, MirRvalue, MirStatement,
    MirTerminator,
};
use super::ty::MirType;

/// Sentinel terminator placed on freshly allocated blocks. Lowering
/// must replace it before [`finish`] is called; [`finish`] asserts on
/// any leftover sentinel.
fn pending_terminator() -> MirTerminator {
    MirTerminator::Unreachable
}

/// In-progress MIR function builder.
pub struct FunctionBuilder {
    name: String,
    origin: String,
    ret_ty: MirType,
    span: Span,
    locals: Vec<MirLocalDecl>,
    blocks: Vec<MirBlock>,
    params: Vec<MirLocal>,
    /// Whether each block has had its real terminator set yet.
    block_set: Vec<bool>,
}

impl FunctionBuilder {
    pub fn new(name: String, origin: String, ret_ty: MirType, span: Span) -> Self {
        Self {
            name,
            origin,
            ret_ty,
            span,
            locals: Vec::new(),
            blocks: Vec::new(),
            params: Vec::new(),
            block_set: Vec::new(),
        }
    }

    /// Declare a parameter local. Must be called before any non-param
    /// locals are allocated so the parameter list lines up with the
    /// caller's argument list.
    pub fn add_param(&mut self, name: String, ty: MirType) -> MirLocal {
        let local = self.fresh_local(name, ty, true);
        self.params.push(local);
        local
    }

    /// Allocate a fresh local for an intermediate value.
    pub fn fresh_temp(&mut self, hint: &str, ty: MirType) -> MirLocal {
        let name = format!("{}_{}", hint, self.locals.len());
        self.fresh_local(name, ty, false)
    }

    /// Allocate a fresh local with a fixed source name.
    pub fn named_local(&mut self, name: String, ty: MirType) -> MirLocal {
        self.fresh_local(name, ty, false)
    }

    fn fresh_local(&mut self, name: String, ty: MirType, is_param: bool) -> MirLocal {
        let idx = self.locals.len() as u32;
        self.locals.push(MirLocalDecl { name, ty, is_param });
        MirLocal(idx)
    }

    /// Allocate a new block. The caller must close it with [`close_block`]
    /// before the function is finished.
    pub fn new_block(&mut self) -> MirBlockId {
        let id = MirBlockId(self.blocks.len() as u32);
        self.blocks.push(MirBlock {
            id,
            statements: Vec::new(),
            terminator: pending_terminator(),
        });
        self.block_set.push(false);
        id
    }

    /// Emit a statement at the tail of `block`.
    pub fn emit(&mut self, block: MirBlockId, stmt: MirStatement) {
        self.blocks[block.0 as usize].statements.push(stmt);
    }

    /// Convenience wrapper around [`emit`] that builds an `Assign`.
    pub fn assign(&mut self, block: MirBlockId, dst: MirLocal, rvalue: MirRvalue) {
        self.emit(block, MirStatement::Assign { dst, rvalue });
    }

    /// Close `block` with `terminator`. Calling this twice on the same
    /// block panics; that always indicates a lowering bug.
    pub fn close_block(&mut self, block: MirBlockId, terminator: MirTerminator) {
        let idx = block.0 as usize;
        if self.block_set[idx] {
            panic!("close_block called twice on bb{}", block.0);
        }
        self.blocks[idx].terminator = terminator;
        self.block_set[idx] = true;
    }

    /// True once `close_block` has been called for `block`.
    pub fn is_closed(&self, block: MirBlockId) -> bool {
        self.block_set[block.0 as usize]
    }

    /// True if `block` is unclosed and contains no statements. The
    /// lowering pass uses this to recognize the "dead block after a
    /// `return`" pattern and skip emitting a redundant unit return.
    pub fn is_empty_open(&self, block: MirBlockId) -> bool {
        let idx = block.0 as usize;
        !self.block_set[idx] && self.blocks[idx].statements.is_empty()
    }

    pub fn locals(&self) -> &[MirLocalDecl] {
        &self.locals
    }

    /// Finalize the builder into a [`MirFunction`]. Any block that was
    /// allocated but never closed is treated as unreachable.
    pub fn finish(mut self, entry: MirBlockId) -> MirFunction {
        for (i, set) in self.block_set.iter().enumerate() {
            if !set {
                self.blocks[i].terminator = MirTerminator::Unreachable;
            }
        }
        MirFunction {
            name: self.name,
            origin: self.origin,
            params: self.params,
            ret_ty: self.ret_ty,
            locals: self.locals,
            blocks: self.blocks,
            entry,
            span: self.span,
        }
    }
}
