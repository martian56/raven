//! Helpers for desugaring rules.
//!
//! Each public function in this module implements one of the rewrite
//! rules described in `docs/v2/specs/hir.md`. They take low-level
//! ingredients (already-lowered HIR sub-trees) and assemble the
//! desugared shape.

use crate::span::Span;
use crate::tycheck::Ty;

use crate::hir::expr::{HirArm, HirBlock, HirExpr, HirExprKind};
use crate::hir::pattern::{HirPattern, HirPatternKind};
use crate::hir::stmt::{HirAssignTarget, HirStmt, HirStmtKind};

/// Build a fresh `HirExpr` with a given kind, type, and span.
pub(crate) fn make_expr(kind: HirExprKind, ty: Ty, span: Span) -> HirExpr {
    HirExpr { kind, ty, span }
}

/// Wrap an expression into a block whose tail is that expression.
pub(crate) fn block_of_tail(expr: HirExpr) -> HirBlock {
    let ty = expr.ty.clone();
    let span = expr.span.clone();
    HirBlock {
        stmts: Vec::new(),
        tail: Some(Box::new(expr)),
        ty,
        span,
    }
}

/// Wrap a unit value as a block. Used for the body of HIR loop arms
/// when the source did not provide one.
#[allow(dead_code)]
pub(crate) fn unit_block(span: Span) -> HirBlock {
    HirBlock {
        stmts: Vec::new(),
        tail: Some(Box::new(HirExpr {
            kind: HirExprKind::Unit,
            ty: Ty::Unit,
            span: span.clone(),
        })),
        ty: Ty::Unit,
        span,
    }
}

/// Build a `let __name: ty = init;` statement.
pub(crate) fn let_stmt(name: &str, ty: Ty, init: HirExpr, span: Span) -> HirStmt {
    HirStmt {
        kind: HirStmtKind::Let {
            name: name.to_string(),
            ty,
            init,
        },
        span,
    }
}

/// Build an identifier reference expression for a synthesized name.
pub(crate) fn ident_expr(name: &str, ty: Ty, span: Span) -> HirExpr {
    HirExpr {
        kind: HirExprKind::Ident(name.to_string()),
        ty,
        span,
    }
}

/// Build a wildcard arm with a body. Used for the default arm of
/// desugared loops.
#[allow(dead_code)]
pub(crate) fn wildcard_arm(body: HirExpr, span: Span) -> HirArm {
    HirArm {
        pattern: HirPattern {
            kind: HirPatternKind::Wildcard,
            span: span.clone(),
        },
        guard: None,
        body,
        span,
    }
}

/// Build an assignment statement `target = value`.
pub(crate) fn assign_stmt(target: HirAssignTarget, value: HirExpr, span: Span) -> HirStmt {
    HirStmt {
        kind: HirStmtKind::Assign { target, value },
        span,
    }
}
