//! High level IR (HIR) for the Raven v2 compiler.
//!
//! The HIR is a desugared form of the AST. Source level sugar (`for`
//! loops, `?` propagation, string interpolation, ranges, compound
//! assignment, single expression function bodies) is rewritten into a
//! smaller core. Every HIR node keeps the originating `Span` and, for
//! expressions, the inferred `Ty` from the type checker.
//!
//! See `docs/v2/specs/hir.md` for the full design.

pub mod decl;
pub mod expr;
pub mod lower;
pub mod pattern;
pub mod pretty;
pub mod stmt;
pub mod ty;

#[cfg(test)]
mod tests;

pub use decl::{HirEnum, HirFn, HirImpl, HirItem, HirItemKind, HirStruct, HirTrait, HirVariant};
pub use expr::{HirArm, HirBinaryOp, HirBlock, HirExpr, HirExprKind, HirUnaryOp, InterpolPart};
pub use lower::lower_file;
pub use pattern::{HirFieldPat, HirLiteralPat, HirPattern, HirPatternKind};
pub use pretty::pretty_program;
pub use stmt::{HirAssignTarget, HirStmt, HirStmtKind};
pub use ty::HirTy;

use crate::span::Span;

/// One Raven source file after HIR lowering.
#[derive(Debug, Clone)]
pub struct HirProgram {
    pub items: Vec<HirItem>,
    pub span: Span,
}
