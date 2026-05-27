//! Expression lowering (HIR -> MIR).
//!
//! Each `lower_*` helper emits whatever statements it needs into the
//! builder's current block and returns an `MirOperand` carrying the
//! computed value. Control-flow expressions (`if`, `match`, `loop`)
//! may also rewrite `LowerCx::current` so the caller continues from
//! the right continuation block.

use crate::hir::expr::{HirBinaryOp, HirBlock, HirExpr, HirExprKind, HirUnaryOp, InterpolPart};

use super::super::ir::{
    MirBinOp, MirBlockId, MirConstant, MirFnRef, MirLocal, MirOperand, MirRvalue, MirStatement,
    MirTerminator, MirUnOp,
};
use super::super::ty::MirType;
use super::{mir_ty, stmt, LoopFrame, LowerCx};

/// Lower an expression to an operand. Side effects appear in the
/// current block. Control-flow constructs may switch the current
/// block before returning.
pub fn lower_expr(cx: &mut LowerCx<'_>, expr: &HirExpr) -> MirOperand {
    let ty = mir_ty(&expr.ty, cx.subst);
    match &expr.kind {
        HirExprKind::Int(i) => MirOperand::Const(MirConstant::Int(*i)),
        HirExprKind::Float(v) => MirOperand::Const(MirConstant::Float(*v)),
        HirExprKind::Bool(b) => MirOperand::Const(MirConstant::Bool(*b)),
        HirExprKind::Str(s) => MirOperand::Const(MirConstant::Str(s.clone())),
        HirExprKind::Char(c) => MirOperand::Const(MirConstant::Char(*c)),
        HirExprKind::CStr(s) => MirOperand::Const(MirConstant::Str(s.clone())),
        HirExprKind::Unit => MirOperand::Const(MirConstant::Unit),
        HirExprKind::Ident(name) => match cx.lookup(name) {
            Some(local) => MirOperand::Copy(local),
            None => {
                // Free name (e.g. callable, constant, or undeclared
                // global). The type checker has already validated the
                // program; for callable references we emit a synthetic
                // zero so codegen has something concrete.
                MirOperand::Const(MirConstant::Unit)
            }
        },
        HirExprKind::SelfValue => match cx.lookup("self") {
            Some(local) => MirOperand::Copy(local),
            None => MirOperand::Const(MirConstant::Unit),
        },
        HirExprKind::Paren(inner) => lower_expr(cx, inner),
        HirExprKind::Block(b) => lower_block(cx, b),
        HirExprKind::Unary { op, operand } => {
            let o = lower_expr(cx, operand);
            let dst = cx.builder.fresh_temp("unary", ty);
            cx.builder
                .assign(cx.current, dst, MirRvalue::UnaryOp(map_unary(*op), o));
            MirOperand::Copy(dst)
        }
        HirExprKind::Binary { op, lhs, rhs } => {
            let l = lower_expr(cx, lhs);
            let r = lower_expr(cx, rhs);
            let dst = cx.builder.fresh_temp("bin", ty);
            cx.builder
                .assign(cx.current, dst, MirRvalue::BinaryOp(map_binary(*op), l, r));
            MirOperand::Copy(dst)
        }
        HirExprKind::Array(items) => {
            let elements: Vec<MirOperand> = items.iter().map(|i| lower_expr(cx, i)).collect();
            let dst = cx.builder.fresh_temp("array", ty.clone());
            cx.builder
                .assign(cx.current, dst, MirRvalue::ArrayLit { ty, elements });
            MirOperand::Copy(dst)
        }
        HirExprKind::StructLit { name: _, fields } => {
            let ops: Vec<MirOperand> = fields.iter().map(|(_, e)| lower_expr(cx, e)).collect();
            let dst = cx.builder.fresh_temp("struct", ty.clone());
            cx.builder
                .assign(cx.current, dst, MirRvalue::StructCreate { ty, fields: ops });
            MirOperand::Copy(dst)
        }
        HirExprKind::Call { callee, args } => {
            let callee_ref = call_ref_from_callee(cx, callee);
            let arg_ops: Vec<MirOperand> = args.iter().map(|a| lower_expr(cx, a)).collect();
            let dst = cx.builder.fresh_temp("call", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::Call {
                    callee: callee_ref,
                    args: arg_ops,
                },
            );
            MirOperand::Copy(dst)
        }
        HirExprKind::MethodCall {
            receiver,
            name,
            args,
        } => {
            let recv = lower_expr(cx, receiver);
            let mut arg_ops = vec![recv];
            for a in args {
                arg_ops.push(lower_expr(cx, a));
            }
            let dst = cx.builder.fresh_temp("mcall", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::Call {
                    callee: MirFnRef {
                        mangled: name.clone(),
                        origin: None,
                    },
                    args: arg_ops,
                },
            );
            MirOperand::Copy(dst)
        }
        HirExprKind::Field { receiver, name } => {
            let base = lower_expr(cx, receiver);
            let index = field_index_from_ty(&receiver.ty, cx, name);
            let dst = cx.builder.fresh_temp("field", ty);
            cx.builder
                .assign(cx.current, dst, MirRvalue::FieldAccess { base, index });
            MirOperand::Copy(dst)
        }
        HirExprKind::Index { receiver, index } => {
            let base = lower_expr(cx, receiver);
            let idx = lower_expr(cx, index);
            let dst = cx.builder.fresh_temp("index", ty);
            cx.builder
                .assign(cx.current, dst, MirRvalue::IndexAccess { base, index: idx });
            MirOperand::Copy(dst)
        }
        HirExprKind::If {
            cond,
            then_block,
            else_block,
        } => lower_if(cx, cond, then_block, else_block.as_ref(), ty),
        HirExprKind::Match { scrutinee, arms } => {
            super::pattern::lower_match(cx, scrutinee, arms, ty)
        }
        HirExprKind::Loop(body) => lower_loop(cx, body, ty),
        HirExprKind::While { cond, body } => lower_while(cx, cond, body, ty),
        HirExprKind::Return(value) => {
            let op = match value {
                Some(v) => lower_expr(cx, v),
                None => MirOperand::Const(MirConstant::Unit),
            };
            if !cx.builder.is_closed(cx.current) {
                cx.builder
                    .close_block(cx.current, MirTerminator::Return(op));
            }
            // Following code is unreachable; start a fresh dead block
            // so subsequent statements still have a target.
            let dead = cx.builder.new_block();
            cx.current = dead;
            MirOperand::Const(MirConstant::Unit)
        }
        HirExprKind::Break(value) => {
            let frame = cx
                .loops
                .last()
                .expect("break outside loop should be a tycheck error");
            let cont = frame.continuation;
            let result = frame.result;
            if let (Some(v), Some(r)) = (value.as_deref(), result) {
                let op = lower_expr(cx, v);
                cx.builder.assign(cx.current, r, MirRvalue::Use(op));
            }
            if !cx.builder.is_closed(cx.current) {
                cx.builder
                    .close_block(cx.current, MirTerminator::Goto(cont));
            }
            let dead = cx.builder.new_block();
            cx.current = dead;
            MirOperand::Const(MirConstant::Unit)
        }
        HirExprKind::Continue => {
            let frame = cx
                .loops
                .last()
                .expect("continue outside loop should be a tycheck error");
            let header = frame.header;
            if !cx.builder.is_closed(cx.current) {
                cx.builder
                    .close_block(cx.current, MirTerminator::Goto(header));
            }
            let dead = cx.builder.new_block();
            cx.current = dead;
            MirOperand::Const(MirConstant::Unit)
        }
        HirExprKind::Interpolate(parts) => lower_interpolate(cx, parts, ty),
        HirExprKind::RangeNew {
            start,
            end,
            inclusive,
        } => {
            let s = lower_expr(cx, start);
            let e = lower_expr(cx, end);
            let inc = MirOperand::Const(MirConstant::Bool(*inclusive));
            let dst = cx.builder.fresh_temp("range", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::Call {
                    callee: MirFnRef {
                        mangled: "__range_new".into(),
                        origin: None,
                    },
                    args: vec![s, e, inc],
                },
            );
            MirOperand::Copy(dst)
        }
        HirExprKind::IterNew(inner) => {
            let v = lower_expr(cx, inner);
            let dst = cx.builder.fresh_temp("iter", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::Call {
                    callee: MirFnRef {
                        mangled: "__iter_new".into(),
                        origin: None,
                    },
                    args: vec![v],
                },
            );
            MirOperand::Copy(dst)
        }
        HirExprKind::IterNext(inner) => {
            let v = lower_expr(cx, inner);
            let dst = cx.builder.fresh_temp("next", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::Call {
                    callee: MirFnRef {
                        mangled: "__iter_next".into(),
                        origin: None,
                    },
                    args: vec![v],
                },
            );
            MirOperand::Copy(dst)
        }
        HirExprKind::OkCtor(inner) => enum_ctor_unary(cx, inner, "Ok", 0, ty),
        HirExprKind::ErrCtor(inner) => enum_ctor_unary(cx, inner, "Err", 1, ty),
        HirExprKind::SomeCtor(inner) => enum_ctor_unary(cx, inner, "Some", 0, ty),
        HirExprKind::NoneCtor => {
            let dst = cx.builder.fresh_temp("none", ty.clone());
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::EnumCreate {
                    ty,
                    variant: 1,
                    payload: Vec::new(),
                },
            );
            MirOperand::Copy(dst)
        }
        HirExprKind::Lambda {
            params: _,
            ret: _,
            body: _,
        } => {
            // Closure capture analysis is deferred (issue #62). For
            // now emit a placeholder operand and a ClosureCreate with
            // no captures so the shape is visible in dumps.
            let dst = cx.builder.fresh_temp("closure", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::ClosureCreate {
                    fn_name: "__closure".into(),
                    captures: Vec::new(),
                },
            );
            MirOperand::Copy(dst)
        }
    }
}

/// Lower a block to a single result operand. Statements are emitted as
/// side effects; the trailing expression (or unit) becomes the result.
pub fn lower_block(cx: &mut LowerCx<'_>, block: &HirBlock) -> MirOperand {
    cx.push_scope();
    for s in &block.stmts {
        stmt::lower_stmt(cx, s);
    }
    let result = match &block.tail {
        Some(tail) => lower_expr(cx, tail),
        None => MirOperand::Const(MirConstant::Unit),
    };
    cx.pop_scope();
    result
}

fn lower_if(
    cx: &mut LowerCx<'_>,
    cond: &HirExpr,
    then_block: &HirBlock,
    else_block: Option<&HirBlock>,
    ty: MirType,
) -> MirOperand {
    let discr = lower_expr(cx, cond);
    let then_bb = cx.builder.new_block();
    let else_bb = cx.builder.new_block();
    let cont_bb = cx.builder.new_block();
    let result = cx.builder.fresh_temp("if", ty);

    cx.builder.close_block(
        cx.current,
        MirTerminator::SwitchInt {
            discriminant: discr,
            targets: vec![(0, else_bb), (1, then_bb)],
            otherwise: else_bb,
        },
    );

    cx.current = then_bb;
    let tv = lower_expr_block(cx, then_block);
    cx.builder.assign(cx.current, result, MirRvalue::Use(tv));
    if !cx.builder.is_closed(cx.current) {
        cx.builder
            .close_block(cx.current, MirTerminator::Goto(cont_bb));
    }

    cx.current = else_bb;
    let ev = match else_block {
        Some(b) => lower_expr_block(cx, b),
        None => MirOperand::Const(MirConstant::Unit),
    };
    cx.builder.assign(cx.current, result, MirRvalue::Use(ev));
    if !cx.builder.is_closed(cx.current) {
        cx.builder
            .close_block(cx.current, MirTerminator::Goto(cont_bb));
    }

    cx.current = cont_bb;
    MirOperand::Copy(result)
}

fn lower_expr_block(cx: &mut LowerCx<'_>, block: &HirBlock) -> MirOperand {
    lower_block(cx, block)
}

fn lower_loop(cx: &mut LowerCx<'_>, body: &HirBlock, ty: MirType) -> MirOperand {
    let header = cx.builder.new_block();
    let cont = cx.builder.new_block();
    let result = cx.builder.fresh_temp("loop", ty);

    if !cx.builder.is_closed(cx.current) {
        cx.builder
            .close_block(cx.current, MirTerminator::Goto(header));
    }

    cx.current = header;
    cx.loops.push(LoopFrame {
        header,
        continuation: cont,
        result: Some(result),
    });
    let _ = lower_block(cx, body);
    if !cx.builder.is_closed(cx.current) {
        cx.builder
            .close_block(cx.current, MirTerminator::Goto(header));
    }
    cx.loops.pop();

    cx.current = cont;
    MirOperand::Copy(result)
}

fn lower_while(cx: &mut LowerCx<'_>, cond: &HirExpr, body: &HirBlock, _ty: MirType) -> MirOperand {
    let header = cx.builder.new_block();
    let body_bb = cx.builder.new_block();
    let cont = cx.builder.new_block();

    if !cx.builder.is_closed(cx.current) {
        cx.builder
            .close_block(cx.current, MirTerminator::Goto(header));
    }

    cx.current = header;
    let c = lower_expr(cx, cond);
    cx.builder.close_block(
        cx.current,
        MirTerminator::SwitchInt {
            discriminant: c,
            targets: vec![(0, cont), (1, body_bb)],
            otherwise: cont,
        },
    );

    cx.current = body_bb;
    cx.loops.push(LoopFrame {
        header,
        continuation: cont,
        result: None,
    });
    let _ = lower_block(cx, body);
    if !cx.builder.is_closed(cx.current) {
        cx.builder
            .close_block(cx.current, MirTerminator::Goto(header));
    }
    cx.loops.pop();

    cx.current = cont;
    MirOperand::Const(MirConstant::Unit)
}

fn lower_interpolate(cx: &mut LowerCx<'_>, parts: &[InterpolPart], ty: MirType) -> MirOperand {
    let mut args: Vec<MirOperand> = Vec::with_capacity(parts.len());
    for p in parts {
        match p {
            InterpolPart::Text(s) => args.push(MirOperand::Const(MirConstant::Str(s.clone()))),
            InterpolPart::Expr(e) => args.push(lower_expr(cx, e)),
        }
    }
    let dst = cx.builder.fresh_temp("interp", ty);
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::Call {
            callee: MirFnRef {
                mangled: "__concat_string".into(),
                origin: None,
            },
            args,
        },
    );
    MirOperand::Copy(dst)
}

fn enum_ctor_unary(
    cx: &mut LowerCx<'_>,
    inner: &HirExpr,
    _ctor_name: &str,
    variant: usize,
    ty: MirType,
) -> MirOperand {
    let payload = vec![lower_expr(cx, inner)];
    let dst = cx.builder.fresh_temp("enum", ty.clone());
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::EnumCreate {
            ty,
            variant,
            payload,
        },
    );
    MirOperand::Copy(dst)
}

/// Turn the callee expression of a `HirExprKind::Call` into a
/// `MirFnRef`. For bare identifiers we just borrow the name; resolving
/// to a `DeclId` and queueing a monomorphization happens in `mono`.
fn call_ref_from_callee(_cx: &mut LowerCx<'_>, callee: &HirExpr) -> MirFnRef {
    match &callee.kind {
        HirExprKind::Ident(name) => MirFnRef {
            mangled: name.clone(),
            origin: None,
        },
        _ => MirFnRef {
            mangled: "__indirect_call".into(),
            origin: None,
        },
    }
}

/// Best-effort field index lookup. The receiver's HIR type tells us
/// which struct to consult; without a generics database here we just
/// emit `0` as a placeholder for indices we cannot resolve. Codegen
/// derives the layout from the struct declaration, not from this hint.
fn field_index_from_ty(_ty: &crate::tycheck::Ty, _cx: &LowerCx<'_>, _name: &str) -> usize {
    0
}

fn map_binary(op: HirBinaryOp) -> MirBinOp {
    match op {
        HirBinaryOp::Add => MirBinOp::Add,
        HirBinaryOp::Sub => MirBinOp::Sub,
        HirBinaryOp::Mul => MirBinOp::Mul,
        HirBinaryOp::Div => MirBinOp::Div,
        HirBinaryOp::Mod => MirBinOp::Mod,
        HirBinaryOp::Eq => MirBinOp::Eq,
        HirBinaryOp::Ne => MirBinOp::Ne,
        HirBinaryOp::Lt => MirBinOp::Lt,
        HirBinaryOp::Le => MirBinOp::Le,
        HirBinaryOp::Gt => MirBinOp::Gt,
        HirBinaryOp::Ge => MirBinOp::Ge,
        HirBinaryOp::And => MirBinOp::And,
        HirBinaryOp::Or => MirBinOp::Or,
        HirBinaryOp::BitAnd => MirBinOp::BitAnd,
        HirBinaryOp::BitOr => MirBinOp::BitOr,
        HirBinaryOp::BitXor => MirBinOp::BitXor,
        HirBinaryOp::Shl => MirBinOp::Shl,
        HirBinaryOp::Shr => MirBinOp::Shr,
    }
}

fn map_unary(op: HirUnaryOp) -> MirUnOp {
    match op {
        HirUnaryOp::Neg => MirUnOp::Neg,
        HirUnaryOp::Not => MirUnOp::Not,
        HirUnaryOp::Ref => MirUnOp::Ref,
    }
}

#[allow(dead_code)]
pub(crate) fn unused_marker(_: MirLocal, _: MirBlockId, _: MirStatement) {}
