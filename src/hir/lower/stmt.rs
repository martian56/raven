//! Lowering AST statements into one or more HIR statements.
//!
//! Most statements lower one to one. Compound assignment (`+=`, etc.)
//! expands to one or more statements, hence the `Vec<HirStmt>` return.

use crate::ast::{AssignOp, Stmt, StmtKind};
use crate::error::RavenError;
use crate::tycheck::Ty;

use crate::hir::expr::{HirBinaryOp, HirExpr, HirExprKind};
use crate::hir::stmt::{HirStmt, HirStmtKind};

use super::expr::{build_plain_assign, lower_assign_target, lower_compound_assign, lower_expr};
use super::LowerCtx;

/// Lower one statement into one or more HIR statements.
pub(crate) fn lower_stmt(stmt: &Stmt, cx: &LowerCtx<'_>) -> Result<Vec<HirStmt>, RavenError> {
    match &stmt.kind {
        StmtKind::Let { name, ty: _, init } => {
            let init = init
                .as_ref()
                .ok_or_else(|| super::ty_error("missing initializer in let", &stmt.span))?;
            let init_ty = cx.ty_at(&init.span);
            let lowered_init = lower_expr(init, &init_ty, cx)?;
            let ty = lowered_init.ty.clone();
            Ok(vec![HirStmt {
                kind: HirStmtKind::Let {
                    name: name.clone(),
                    ty,
                    init: lowered_init,
                },
                span: stmt.span.clone(),
            }])
        }
        StmtKind::Return(value) => {
            let payload = match value {
                Some(v) => Some(Box::new(lower_expr(v, &Ty::Error, cx)?)),
                None => None,
            };
            let ret = HirExpr {
                kind: HirExprKind::Return(payload),
                ty: Ty::Error,
                span: stmt.span.clone(),
            };
            Ok(vec![HirStmt {
                kind: HirStmtKind::Expr(ret),
                span: stmt.span.clone(),
            }])
        }
        StmtKind::Break(value) => {
            let payload = match value {
                Some(v) => Some(Box::new(lower_expr(v, &Ty::Error, cx)?)),
                None => None,
            };
            let br = HirExpr {
                kind: HirExprKind::Break(payload),
                ty: Ty::Error,
                span: stmt.span.clone(),
            };
            Ok(vec![HirStmt {
                kind: HirStmtKind::Expr(br),
                span: stmt.span.clone(),
            }])
        }
        StmtKind::Continue => {
            let c = HirExpr {
                kind: HirExprKind::Continue,
                ty: Ty::Error,
                span: stmt.span.clone(),
            };
            Ok(vec![HirStmt {
                kind: HirStmtKind::Expr(c),
                span: stmt.span.clone(),
            }])
        }
        StmtKind::Defer(e) => {
            let lowered = lower_expr(e, &Ty::Unit, cx)?;
            Ok(vec![HirStmt {
                kind: HirStmtKind::Defer(lowered),
                span: stmt.span.clone(),
            }])
        }
        StmtKind::Spawn(e) => {
            // The operand is a goroutine body: a `fun() -> Unit` closure.
            let expected = Ty::Function {
                params: Vec::new(),
                ret: Box::new(Ty::Unit),
            };
            let lowered = lower_expr(e, &expected, cx)?;
            Ok(vec![HirStmt {
                kind: HirStmtKind::Spawn(lowered),
                span: stmt.span.clone(),
            }])
        }
        StmtKind::Assign { target, op, value } => match op {
            AssignOp::Assign => {
                let tgt = lower_assign_target(target, cx)?;
                let target_ty = cx.ty_at(&target.span);
                let val = lower_expr(value, &target_ty, cx)?;
                Ok(vec![build_plain_assign(tgt, val, stmt.span.clone())])
            }
            other => {
                let hop = compound_to_binop(*other);
                lower_compound_assign(target, hop, value, &stmt.span, cx)
            }
        },
        StmtKind::Expr(e) => {
            let lowered = lower_expr(e, &Ty::Error, cx)?;
            Ok(vec![HirStmt {
                kind: HirStmtKind::Expr(lowered),
                span: stmt.span.clone(),
            }])
        }
    }
}

fn compound_to_binop(op: AssignOp) -> HirBinaryOp {
    match op {
        AssignOp::Add => HirBinaryOp::Add,
        AssignOp::Sub => HirBinaryOp::Sub,
        AssignOp::Mul => HirBinaryOp::Mul,
        AssignOp::Div => HirBinaryOp::Div,
        AssignOp::Mod => HirBinaryOp::Mod,
        AssignOp::BitAnd => HirBinaryOp::BitAnd,
        AssignOp::BitOr => HirBinaryOp::BitOr,
        AssignOp::BitXor => HirBinaryOp::BitXor,
        AssignOp::Shl => HirBinaryOp::Shl,
        AssignOp::Shr => HirBinaryOp::Shr,
        AssignOp::Assign => HirBinaryOp::Add, // unreachable: handled by caller
    }
}
