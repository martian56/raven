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
        HirExprKind::CStr(s) => MirOperand::Const(MirConstant::CStr(s.clone())),
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
        HirExprKind::StructLit { name, fields } => lower_struct_lit(cx, name, fields, ty),
        HirExprKind::Call { callee, args } => {
            // A callee that is an in-scope local of function type is a
            // closure value: dispatch indirectly through its Closure
            // object. Any other callee is a direct call by symbol.
            if is_closure_value_callee(cx, callee) {
                return lower_closure_call(cx, callee, args, ty);
            }
            let callee_ref = call_ref_from_callee(cx, callee, args);
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
        } => lower_method_call(cx, receiver, name, args, ty),
        HirExprKind::DynCoerce {
            value,
            trait_name,
            methods,
            concrete_ty,
        } => {
            let v = lower_expr(cx, value);
            let concrete = mir_ty(concrete_ty, cx.subst);
            let dst = cx.builder.fresh_temp("dyn", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::DynCoerce {
                    value: v,
                    concrete_ty: concrete,
                    trait_name: trait_name.clone(),
                    methods: methods.clone(),
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
            // A return escapes every enclosing block, so run all pending
            // deferred expressions in reverse order. The return value was
            // already evaluated above, matching Go: the result is computed
            // first, then defers run, then the function returns. Emitting
            // them here (before the Return terminator) also places them
            // before codegen's GC leave-frame epilogue, so deferred code
            // can still touch rooted GC locals.
            cx.emit_all_defers();
            if !cx.builder.is_closed(cx.current) {
                cx.builder
                    .close_block(cx.current, MirTerminator::Return(op));
            }
            // Following code is unreachable; start a fresh dead block
            // so subsequent statements still have a target.
            let dead = cx.builder.new_block();
            cx.current = dead;
            cx.diverged = true;
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
            cx.diverged = true;
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
            cx.diverged = true;
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
                    payload_tys: Vec::new(),
                },
            );
            MirOperand::Copy(dst)
        }
        HirExprKind::Lambda { params, ret, body } => {
            // Run capture analysis, lift the body into a standalone
            // function, and emit a ClosureCreate that allocates the
            // closure and copies each captured value into the env.
            let rvalue = super::closure::lower_lambda(cx, params, ret, body);
            let dst = cx.builder.fresh_temp("closure", ty);
            cx.builder.assign(cx.current, dst, rvalue);
            MirOperand::Copy(dst)
        }
    }
}

/// Lower a block to a single result operand. Statements are emitted as
/// side effects; the trailing expression (or unit) becomes the result.
pub fn lower_block(cx: &mut LowerCx<'_>, block: &HirBlock) -> MirOperand {
    cx.push_scope();
    // Defers are function-scoped at the body level: a `defer` registered
    // in a nested block runs when that block exits, and the mark records
    // where this block's own defers begin so its normal-exit flush does
    // not disturb defers owned by an enclosing block.
    let defer_mark = cx.defer_mark();
    for s in &block.stmts {
        stmt::lower_stmt(cx, s);
    }
    let result = match &block.tail {
        Some(tail) => lower_expr(cx, tail),
        None => MirOperand::Const(MirConstant::Unit),
    };
    // Run the defers this block registered, in reverse order, on the
    // normal fall-through exit. When control already diverged (a `return`,
    // `break`, or `continue` closed the block and rolled a fresh dead
    // block), the normal exit is unreachable: the escaping statement
    // already emitted the defers it needed, so skip the flush rather than
    // populate the dead block. Then drop them so an enclosing block does
    // not re-run them on its own exit.
    if !cx.diverged {
        cx.emit_defers_from(defer_mark);
    }
    cx.defers.truncate(defer_mark);
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
    // The merge block is reachable even if one branch diverged.
    cx.diverged = false;
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
    cx.diverged = false;
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
    cx.diverged = false;
    MirOperand::Const(MirConstant::Unit)
}

/// Desugar an interpolated string into a left-folded chain of runtime
/// string concatenations. Each part is first turned into a heap `String`
/// operand: a literal-text part is a string constant (codegen promotes
/// it to a heap String), and an embedded-expression part is converted
/// with the per-type `to_string` intrinsic selected from its static
/// type (a `String` part needs no conversion). The parts are then folded
/// with `__raven_str_concat` so
/// `["a", x, "b"]` becomes `concat(concat("a", to_string(x)), "b")`.
///
/// An empty interpolation, or one with no parts, still yields a valid
/// (empty) `String`.
fn lower_interpolate(cx: &mut LowerCx<'_>, parts: &[InterpolPart], ty: MirType) -> MirOperand {
    // Build the per-part String operands first.
    let mut operands: Vec<MirOperand> = Vec::with_capacity(parts.len());
    for p in parts {
        match p {
            InterpolPart::Text(s) => operands.push(MirOperand::Const(MirConstant::Str(s.clone()))),
            InterpolPart::Expr(e) => operands.push(stringify_part(cx, e)),
        }
    }

    // No parts: produce an empty heap String.
    let Some(first) = operands.first().cloned() else {
        let dst = cx.builder.fresh_temp("interp", ty);
        cx.builder.assign(
            cx.current,
            dst,
            MirRvalue::Use(MirOperand::Const(MirConstant::Str(String::new()))),
        );
        return MirOperand::Copy(dst);
    };

    // A single part is already a String; bind it to a temp so the result
    // is uniformly a local holding a heap String.
    if operands.len() == 1 {
        let dst = cx.builder.fresh_temp("interp", ty);
        cx.builder.assign(cx.current, dst, MirRvalue::Use(first));
        return MirOperand::Copy(dst);
    }

    // Left-fold: acc = concat(acc, next).
    let mut acc = first;
    for next in operands.into_iter().skip(1) {
        let dst = cx.builder.fresh_temp("concat", MirType::Str);
        cx.builder.assign(
            cx.current,
            dst,
            MirRvalue::Call {
                callee: MirFnRef {
                    mangled: super::super::intrinsics::STR_CONCAT.into(),
                    origin: None,
                },
                args: vec![acc, next],
            },
        );
        acc = MirOperand::Copy(dst);
    }
    acc
}

/// Lower an embedded interpolation expression and convert it to a heap
/// `String`. A `String`-typed expression is used as-is; the other
/// interpolatable scalars (`Int`, `Bool`, `Float`, `Char`) are routed
/// through their `to_string` runtime intrinsic. The type checker has
/// already rejected any other type, so an unexpected type here is a
/// lowering bug; we fall back to using the value directly.
fn stringify_part(cx: &mut LowerCx<'_>, e: &HirExpr) -> MirOperand {
    let value = lower_expr(cx, e);
    let part_ty = mir_ty(&e.ty, cx.subst);
    let intrinsic = match part_ty {
        MirType::Str => return value,
        MirType::Int => super::super::intrinsics::INT_TO_STRING,
        MirType::Bool => super::super::intrinsics::BOOL_TO_STRING,
        MirType::Float => super::super::intrinsics::FLOAT_TO_STRING,
        MirType::Char => super::super::intrinsics::CHAR_TO_STRING,
        _ => return value,
    };
    let dst = cx.builder.fresh_temp("tostr", MirType::Str);
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::Call {
            callee: MirFnRef {
                mangled: intrinsic.into(),
                origin: None,
            },
            args: vec![value],
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
    let payload_ty = mir_ty(&inner.ty, cx.subst);
    let payload = vec![lower_expr(cx, inner)];
    let dst = cx.builder.fresh_temp("enum", ty.clone());
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::EnumCreate {
            ty,
            variant,
            payload,
            payload_tys: vec![payload_ty],
        },
    );
    MirOperand::Copy(dst)
}

/// Lower a struct literal. Reorders the source-order field initializers
/// into the struct's declaration order so the field slot offsets match
/// what `FieldAccess` reads, and records each field's concrete type so
/// the back-end can build the GC pointer descriptor.
fn lower_struct_lit(
    cx: &mut LowerCx<'_>,
    name: &str,
    fields: &[(String, HirExpr)],
    ty: MirType,
) -> MirOperand {
    // Declaration order from the struct table, when available. Generic
    // structs are out of scope for the MVP, so the source name keys the
    // single monomorphic declaration directly.
    let decl_order: Option<Vec<(String, MirType)>> = cx.decls.structs.get(name).map(|s| {
        s.fields
            .iter()
            .map(|(fname, fty, _)| (fname.clone(), mir_ty(fty, cx.subst)))
            .collect()
    });

    let (ops, field_tys): (Vec<MirOperand>, Vec<MirType>) = match decl_order {
        Some(order) => {
            // For each declared field, find the matching initializer and
            // lower it. The type checker has already verified every field
            // is initialized exactly once.
            let mut ops = Vec::with_capacity(order.len());
            let mut tys = Vec::with_capacity(order.len());
            for (fname, fty) in &order {
                let init = fields
                    .iter()
                    .find(|(n, _)| n == fname)
                    .map(|(_, e)| e)
                    .expect("type checker guarantees every field is initialized");
                ops.push(lower_expr(cx, init));
                tys.push(fty.clone());
            }
            (ops, tys)
        }
        None => {
            // No declaration in scope (should not happen for a checked
            // program). Fall back to source order with operand-derived
            // types so codegen still has something concrete.
            let mut ops = Vec::with_capacity(fields.len());
            let mut tys = Vec::with_capacity(fields.len());
            for (_, e) in fields {
                tys.push(mir_ty(&e.ty, cx.subst));
                ops.push(lower_expr(cx, e));
            }
            (ops, tys)
        }
    };

    let dst = cx.builder.fresh_temp("struct", ty.clone());
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::StructCreate {
            ty,
            fields: ops,
            field_tys,
        },
    );
    MirOperand::Copy(dst)
}

/// Turn the callee expression of a `HirExprKind::Call` into a
/// `MirFnRef`. A bare identifier naming a generic free function is
/// specialized here: the callee's declared parameter types (which carry
/// `Ty::Param`) are matched against the concrete argument types to build
/// the substitution, the per-instantiation symbol is computed, and the
/// instantiation is queued for the monomorphizer. A non-generic callee
/// keeps its source name.
fn call_ref_from_callee(cx: &mut LowerCx<'_>, callee: &HirExpr, args: &[HirExpr]) -> MirFnRef {
    let HirExprKind::Ident(name) = &callee.kind else {
        return MirFnRef {
            mangled: "__indirect_call".into(),
            origin: None,
        };
    };
    let Some(entry) = cx.decls.functions.get(name).cloned() else {
        // Not a known free function (a builtin intrinsic or constructor):
        // keep the bare name for the back end to recognize.
        return MirFnRef {
            mangled: name.clone(),
            origin: None,
        };
    };
    if entry.generic_params.is_empty() {
        return MirFnRef {
            mangled: name.clone(),
            origin: None,
        };
    }
    // Build the substitution by matching each callee parameter type
    // (which carries the callee's own `Ty::Param`s) against the concrete
    // argument type. The argument type has the enclosing function's
    // substitution applied first so a caller's own `Ty::Param` is already
    // ground; the callee's `Ty::Param`s are left intact so `match_param`
    // can bind them. The two parameter spaces never collide because a
    // `ParamId` carries its owner span.
    let mut subst: super::SubstMap = super::SubstMap::new();
    for (decl_ty, arg) in entry.params.iter().zip(args.iter()) {
        let got = super::substitute(&arg.ty, cx.subst);
        match_param(decl_ty, &got, &mut subst);
    }
    let mangled = super::mono_symbol(name, &entry.generic_params, &subst);
    cx.pending_calls.push((entry.decl, subst));
    MirFnRef {
        mangled,
        origin: None,
    }
}

/// Match a declared type against a concrete type, recording the concrete
/// substitute for every `Ty::Param` encountered. A structural walk: when
/// the declared side is a `Ty::Param`, bind it to the concrete side;
/// otherwise descend into matching shapes. Mismatched shapes are ignored
/// (a checked program never produces one, and a partial match still
/// yields a usable substitution).
fn match_param(
    decl: &crate::tycheck::Ty,
    concrete: &crate::tycheck::Ty,
    out: &mut super::SubstMap,
) {
    use crate::tycheck::Ty;
    match (decl, concrete) {
        (Ty::Param(p), c) => {
            out.entry(p.clone()).or_insert_with(|| c.clone());
        }
        (Ty::Option(a), Ty::Option(b))
        | (Ty::List(a), Ty::List(b))
        | (Ty::SelfTy(a), Ty::SelfTy(b)) => match_param(a, b, out),
        (Ty::Result(a1, a2), Ty::Result(b1, b2)) => {
            match_param(a1, b1, out);
            match_param(a2, b2, out);
        }
        (Ty::Struct { args: a, .. }, Ty::Struct { args: b, .. })
        | (Ty::Enum { args: a, .. }, Ty::Enum { args: b, .. }) => {
            for (x, y) in a.iter().zip(b.iter()) {
                match_param(x, y, out);
            }
        }
        (
            Ty::Function {
                params: ap,
                ret: ar,
            },
            Ty::Function {
                params: bp,
                ret: br,
            },
        ) => {
            for (x, y) in ap.iter().zip(bp.iter()) {
                match_param(x, y, out);
            }
            match_param(ar, br, out);
        }
        _ => {}
    }
}

/// Decide whether a call's callee is a closure value to dispatch
/// indirectly. A callee is a closure value when its type is a function
/// type and it is not a bare reference to a top-level function. A bare
/// identifier that is in scope as a local or parameter (so `cx.lookup`
/// finds it) and has a function type is a closure value; an identifier
/// that does not resolve to an in-scope local is a top-level function
/// reference dispatched by symbol.
fn is_closure_value_callee(cx: &LowerCx<'_>, callee: &HirExpr) -> bool {
    if !matches!(callee.ty, crate::tycheck::Ty::Function { .. }) {
        return false;
    }
    match &callee.kind {
        // Only a local or parameter of function type is a closure value.
        // A top-level function name is not in `cx.scopes`.
        HirExprKind::Ident(name) => cx.lookup(name).is_some(),
        // Any other expression of function type (a returned closure, a
        // field holding a closure, an element of a list, ...) is a
        // closure value.
        _ => true,
    }
}

/// Lower a closure-value call: evaluate the callee to a `Closure`
/// pointer, then dispatch indirectly through it. The runtime loads the
/// function pointer and capture env from the Closure object; the env is
/// passed as the leading argument followed by the user arguments.
fn lower_closure_call(
    cx: &mut LowerCx<'_>,
    callee: &HirExpr,
    args: &[HirExpr],
    ty: MirType,
) -> MirOperand {
    let closure = lower_expr(cx, callee);
    let mut arg_ops = Vec::with_capacity(args.len());
    let mut param_tys = Vec::with_capacity(args.len());
    for a in args {
        param_tys.push(mir_ty(&a.ty, cx.subst));
        arg_ops.push(lower_expr(cx, a));
    }
    let dst = cx.builder.fresh_temp("ccall", ty.clone());
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::ClosureCall {
            closure,
            args: arg_ops,
            param_tys,
            ret_ty: ty,
        },
    );
    MirOperand::Copy(dst)
}

/// Lower a method call. A concrete receiver dispatches statically to the
/// per-type method symbol `<RecvType>$<method>`. A `dyn Trait` receiver
/// dispatches virtually through the receiver's vtable.
fn lower_method_call(
    cx: &mut LowerCx<'_>,
    receiver: &HirExpr,
    name: &str,
    args: &[HirExpr],
    ty: MirType,
) -> MirOperand {
    let recv_ty = mir_ty(&receiver.ty, cx.subst);
    if let MirType::Dyn { methods, .. } = &recv_ty {
        // Virtual dispatch: the slot index is the method's position in the
        // trait's declaration order, which the dyn type carries.
        let slot = methods.iter().position(|m| m == name).unwrap_or(0);
        let recv = lower_expr(cx, receiver);
        let mut arg_ops = Vec::with_capacity(args.len());
        let mut param_tys = Vec::with_capacity(args.len());
        for a in args {
            param_tys.push(mir_ty(&a.ty, cx.subst));
            arg_ops.push(lower_expr(cx, a));
        }
        let dst = cx.builder.fresh_temp("vcall", ty.clone());
        cx.builder.assign(
            cx.current,
            dst,
            MirRvalue::VirtualCall {
                receiver: recv,
                slot,
                args: arg_ops,
                param_tys,
                ret_ty: ty,
            },
        );
        return MirOperand::Copy(dst);
    }

    // Static dispatch: build the per-type method symbol from the receiver
    // type so it matches the impl method's definition symbol.
    let symbol = recv_ty.method_symbol(name);
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
                mangled: symbol,
                origin: None,
            },
            args: arg_ops,
        },
    );
    MirOperand::Copy(dst)
}

/// Resolve a field's slot index from the receiver's struct type and the
/// field name. The index is the field's position in declaration order,
/// which matches the slot offset the struct constructor writes. Falls
/// back to `0` only when the receiver is not a known struct (which a
/// checked program never produces).
fn field_index_from_ty(ty: &crate::tycheck::Ty, cx: &LowerCx<'_>, name: &str) -> usize {
    use crate::tycheck::Ty;
    let struct_name = match ty {
        Ty::Struct { name, .. } => Some(name.as_str()),
        Ty::SelfTy(inner) => match inner.as_ref() {
            Ty::Struct { name, .. } => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    };
    if let Some(sname) = struct_name {
        if let Some(decl) = cx.decls.structs.get(sname) {
            if let Some(idx) = decl.fields.iter().position(|(fname, _, _)| fname == name) {
                return idx;
            }
        }
    }
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
