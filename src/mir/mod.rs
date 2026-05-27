//! Mid-level IR (MIR) for the Raven v2 compiler.
//!
//! The MIR is a control-flow graph of basic blocks with named locals.
//! Every operation reads from a local and writes to a local; nested
//! expressions are gone. Generic functions are monomorphized once per
//! distinct concrete type-argument tuple reachable from the program
//! roots.
//!
//! See `docs/v2/specs/mir.md` for the full design.

pub mod builder;
pub mod ir;
pub mod lower;
pub mod mono;
pub mod pretty;
pub mod ty;

#[cfg(test)]
mod tests;

pub use ir::{
    MirBinOp, MirBlock, MirBlockId, MirConstant, MirFnRef, MirFunction, MirLocal, MirLocalDecl,
    MirOperand, MirProgram, MirRvalue, MirStatement, MirTerminator, MirUnOp,
};
pub use pretty::pretty_program;
pub use ty::MirType;

use crate::error::RavenError;
use crate::hir::HirProgram;

/// Lower an entire HIR program to MIR, monomorphizing every reachable
/// generic function.
pub fn lower_program(hir: &HirProgram) -> Result<MirProgram, RavenError> {
    mono::monomorphize(hir)
}
