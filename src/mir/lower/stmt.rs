//! Statement lowering (HIR -> MIR).

use crate::hir::expr::{HirBlock, HirExpr};
use crate::hir::stmt::{HirAssignTarget, HirStmt, HirStmtKind};
use crate::tycheck::Ty;

use super::super::ir::{MirFnRef, MirOperand, MirRvalue};
use super::super::ty::MirType;
use super::{mir_ty, LowerCx};

/// Lower one HIR statement into the current block.
pub fn lower_stmt(cx: &mut LowerCx<'_>, stmt: &HirStmt) {
    match &stmt.kind {
        HirStmtKind::Let { name, ty, init } => {
            let mty = mir_ty(ty, cx.subst);
            let local = cx.builder.named_local(name.clone(), mty);
            let value = super::expr::lower_expr(cx, init);
            cx.builder.assign(cx.current, local, MirRvalue::Use(value));
            cx.bind(name.clone(), local);
        }
        HirStmtKind::Expr(e) => {
            let _ = super::expr::lower_expr(cx, e);
        }
        HirStmtKind::Assign { target, value } => match target {
            HirAssignTarget::Ident { name, .. } => {
                let v = super::expr::lower_expr(cx, value);
                if let Some(local) = cx.lookup(name) {
                    cx.builder.assign(cx.current, local, MirRvalue::Use(v));
                }
            }
            HirAssignTarget::Field { recv, name } => {
                // `recv.name = value` lowers to a real field store: the
                // back end writes `value` into the struct's field slot at
                // `index`, the same slot a `FieldAccess` reads. The slot
                // index comes from the receiver's struct declaration order,
                // identical to the field-read lowering.
                let index = super::expr::field_index_from_ty(&recv.ty, cx, name);
                let base = super::expr::lower_expr(cx, recv);
                let v = super::expr::lower_expr(cx, value);
                cx.builder.store_field(cx.current, base, index, v);
            }
            HirAssignTarget::Index { recv, index } => {
                // `recv[index] = value` lowers to a real element store:
                // the back end bounds-checks `index` and writes `value`
                // into the list slot, mirroring the index-read lowering.
                let base = super::expr::lower_expr(cx, recv);
                let idx = super::expr::lower_expr(cx, index);
                let v = super::expr::lower_expr(cx, value);
                cx.builder.store_index(cx.current, base, idx, v);
            }
        },
        HirStmtKind::Defer(e) => lower_defer(cx, e),
    }
}

/// Lower `defer expr` to a runtime defer-frame registration.
///
/// The deferred expression is lifted into a zero-argument thunk closure
/// (a lambda `fun() { expr }` capturing the free variables `expr` reads),
/// then pushed onto the current call frame's defer stack with
/// `__defer_push`. Pushing at the point the `defer` statement executes
/// keeps the semantics reached-only and the order dynamic; the function
/// epilogue runs the parked thunks in LIFO order at every return. See
/// `docs/v2/specs/defer.md`.
fn lower_defer(cx: &mut LowerCx<'_>, e: &HirExpr) {
    cx.builder.mark_has_defer();

    // Wrap the deferred expression as a statement so the thunk body
    // evaluates it for side effects and yields unit, matching the thunk's
    // `() -> Unit` signature.
    let body = HirBlock {
        stmts: vec![HirStmt {
            kind: HirStmtKind::Expr(e.clone()),
            span: e.span.clone(),
        }],
        tail: None,
        ty: Ty::Unit,
        span: e.span.clone(),
    };
    let ret = Ty::Unit;
    let rvalue = super::closure::lower_lambda(cx, &[], &ret, &body);

    let closure_ty = MirType::Function {
        params: Vec::new(),
        ret: Box::new(MirType::Unit),
    };
    let thunk = cx.builder.fresh_temp("defer_thunk", closure_ty);
    cx.builder.assign(cx.current, thunk, rvalue);
    let push_dst = cx.builder.fresh_temp("defer_push", MirType::Unit);
    cx.builder.assign(
        cx.current,
        push_dst,
        MirRvalue::Call {
            callee: MirFnRef {
                mangled: crate::codegen::intrinsics::DEFER_PUSH_FN.to_string(),
                origin: None,
            },
            args: vec![MirOperand::Copy(thunk)],
        },
    );
}

/// Lower a block and return its result operand.
pub fn lower_block(cx: &mut LowerCx<'_>, block: &HirBlock) -> MirOperand {
    super::expr::lower_block(cx, block)
}
