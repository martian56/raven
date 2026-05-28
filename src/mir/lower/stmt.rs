//! Statement lowering (HIR -> MIR).

use crate::hir::expr::HirBlock;
use crate::hir::stmt::{HirAssignTarget, HirStmt, HirStmtKind};

use super::super::ir::{MirOperand, MirRvalue};
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
        HirStmtKind::Defer(e) => {
            // Register the deferred expression. It is not emitted here:
            // `lower_block` flushes it (in reverse order) when its
            // enclosing block exits, and any `return` that escapes the
            // block emits it first. See `docs/v2/specs/defer.md`.
            cx.defers.push(e.clone());
        }
    }
}

/// Lower a block and return its result operand.
pub fn lower_block(cx: &mut LowerCx<'_>, block: &HirBlock) -> MirOperand {
    super::expr::lower_block(cx, block)
}
