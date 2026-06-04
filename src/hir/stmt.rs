//! Statements appearing inside HIR blocks.
//!
//! Statements in HIR are deliberately small: let-binding, expression,
//! assignment, defer, and a control-flow effect. Return/break/continue
//! live on `HirExpr` so they can appear in any expression position
//! produced by desugaring (`?` lowering emits `return Err(__e)` as an
//! expression, for instance).

use crate::span::Span;

use super::expr::HirExpr;
use super::ty::HirTy;

/// A statement with its source span.
#[derive(Debug, Clone)]
pub struct HirStmt {
    pub kind: HirStmtKind,
    pub span: Span,
}

/// Statement node kinds.
#[derive(Debug, Clone)]
pub enum HirStmtKind {
    /// `let name: ty = init`. After lowering, every `let` has an
    /// initializer (uninitialized module level lets are represented as
    /// items, not statements).
    Let {
        name: String,
        ty: HirTy,
        init: HirExpr,
    },
    /// A bare expression evaluated for its side effects.
    Expr(HirExpr),
    /// `target = value`. Compound assignment has been desugared, so the
    /// only operator here is plain assignment.
    Assign {
        target: HirAssignTarget,
        value: HirExpr,
    },
    /// `defer expr`. Schedules `expr` to run at scope exit; lowered
    /// further by a later pass (issue #68).
    Defer(HirExpr),
    /// `spawn expr`. Starts a goroutine running the `fun() -> Unit`
    /// closure `expr` produces; MIR lowering emits a `raven_go_spawn`
    /// call. See `docs/v2/specs/concurrency.md`.
    Spawn(HirExpr),
}

/// Where an assignment is writing. Compound-assignment lowering can
/// produce any of the variants.
#[derive(Debug, Clone)]
pub enum HirAssignTarget {
    /// Plain identifier `name = value`.
    Ident { name: String, span: Span },
    /// A mutable module-level global `name = value`, written by its mangled
    /// symbol name. The store goes to the global's data slot.
    Global { name: String },
    /// Field `recv.name = value`.
    Field { recv: HirExpr, name: String },
    /// Indexed `recv[index] = value`.
    Index { recv: HirExpr, index: HirExpr },
}
