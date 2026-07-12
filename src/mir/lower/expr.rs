//! Expression lowering (HIR -> MIR).
//!
//! Each `lower_*` helper emits whatever statements it needs into the
//! builder's current block and returns an `MirOperand` carrying the
//! computed value. Control-flow expressions (`if`, `match`, `loop`)
//! may also rewrite `LowerCx::current` so the caller continues from
//! the right continuation block.

use crate::hir::expr::{
    HirBinaryOp, HirBlock, HirExpr, HirExprKind, HirUnaryOp, InterpolPart, PtrBuiltinOp,
    ReflectBuiltinOp,
};

use super::super::ir::{
    ListMethodOp, MirBinOp, MirBlockId, MirConstant, MirFnRef, MirLocal, MirOperand, MirRvalue,
    MirStatement, MirTerminator, MirUnOp, ReflectField, ReflectType,
};
use super::super::ty::{MirFfiTy, MirType};
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
                // A free name of function type used as a value is a
                // reference to a top-level function. When the surrounding
                // context expects a C function pointer this becomes the
                // function's address; the type checker has already
                // verified the function is non-generic and C-FFI typed.
                // Generic free functions cannot be used as a callback (no
                // concrete C ABI), so only non-generic names take an
                // address; any other free name keeps the synthetic zero.
                if let MirType::Function { .. } = &ty {
                    if cx
                        .decls
                        .functions
                        .get(name)
                        .map(|e| e.generic_params.is_empty())
                        .unwrap_or(false)
                    {
                        let dst = cx
                            .builder
                            .fresh_temp("fnaddr", MirType::Ffi(MirFfiTy::CFnPtr));
                        cx.builder.assign(
                            cx.current,
                            dst,
                            MirRvalue::FnAddr {
                                mangled: name.clone(),
                            },
                        );
                        return MirOperand::Copy(dst);
                    }
                }
                MirOperand::Const(MirConstant::Unit)
            }
        },
        HirExprKind::GlobalGet(name) => {
            // Read a mutable module-level global from its data slot.
            let dst = cx.builder.fresh_temp("global", ty.clone());
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::GlobalLoad {
                    name: name.clone(),
                    ty,
                },
            );
            MirOperand::Copy(dst)
        }
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
            // `&&` and `||` short-circuit: the right operand only runs when the
            // left does not already decide the result. Lower them to branches
            // rather than a plain binary op so a guard like
            // `i < xs.len() && xs[i] == x` never evaluates the index when the
            // bound check fails.
            if matches!(op, HirBinaryOp::And | HirBinaryOp::Or) {
                return lower_short_circuit(cx, matches!(op, HirBinaryOp::And), lhs, rhs, ty);
            }
            // `==`/`!=` on `String` compare contents, not object identity,
            // so route them through the runtime byte-equality intrinsic.
            // `!=` negates the equality result. Every other operand type
            // (and every other operator) keeps the value compare below.
            if matches!(op, HirBinaryOp::Eq | HirBinaryOp::Ne)
                && mir_ty(&lhs.ty, cx.subst) == MirType::Str
            {
                return lower_string_eq(cx, *op, lhs, rhs, ty);
            }
            // Ordering on `String` compares contents lexicographically:
            // `raven_string_cmp` returns -1/0/1 and the operator compares
            // that against 0.
            if matches!(
                op,
                HirBinaryOp::Lt | HirBinaryOp::Le | HirBinaryOp::Gt | HirBinaryOp::Ge
            ) && mir_ty(&lhs.ty, cx.subst) == MirType::Str
            {
                return lower_string_cmp(cx, *op, lhs, rhs, ty);
            }
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
        HirExprKind::StructLit { name, fields } => lower_struct_lit(cx, name, fields, &expr.ty, ty),
        HirExprKind::Call {
            callee,
            args,
            type_args,
        } => {
            // A callee that is an in-scope local of function type is a
            // closure value: dispatch indirectly through its Closure
            // object. Any other callee is a direct call by symbol.
            if is_closure_value_callee(cx, callee) {
                return lower_closure_call(cx, callee, args, ty);
            }
            let callee_ref = call_ref_from_callee(cx, callee, args, type_args, &expr.ty);
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
        } => lower_method_call(cx, receiver, name, args, ty, &expr.ty),
        HirExprKind::AssocCall {
            self_ty,
            name,
            args,
        } => lower_assoc_call(cx, self_ty, name, args, ty, &expr.ty),
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
            // The return value is evaluated above. Deferred thunks run in
            // the function epilogue codegen emits at the `Return`
            // terminator (after the value, before leaving the GC frame),
            // so no defer work is emitted here. See `docs/v2/specs/defer.md`.
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
        HirExprKind::EnumCreate { variant, args } => {
            let mut payload = Vec::with_capacity(args.len());
            let mut payload_tys = Vec::with_capacity(args.len());
            for a in args {
                payload_tys.push(mir_ty(&a.ty, cx.subst));
                payload.push(lower_expr(cx, a));
            }
            let dst = cx.builder.fresh_temp("enum", ty.clone());
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::EnumCreate {
                    ty,
                    variant: *variant,
                    payload,
                    payload_tys,
                },
            );
            MirOperand::Copy(dst)
        }
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
        HirExprKind::FnClosure { name } => {
            // A named top-level function used as a value: wrap it in a
            // zero-capture closure so it has the uniform closure
            // representation a higher-order callee expects. A generic or
            // unknown name has no concrete shim; fall back to the synthetic
            // unit a bare free name produced before.
            match super::closure::lower_fn_closure(cx, name, &expr.span) {
                Some(rvalue) => {
                    let dst = cx.builder.fresh_temp("fnclosure", ty);
                    cx.builder.assign(cx.current, dst, rvalue);
                    MirOperand::Copy(dst)
                }
                None => MirOperand::Const(MirConstant::Unit),
            }
        }
        HirExprKind::FnRef(name) => {
            // A resolved function reference is consumed directly in callee
            // position; in value position it behaves like a named-function
            // value (a zero-capture closure).
            match super::closure::lower_fn_closure(cx, name, &expr.span) {
                Some(rvalue) => {
                    let dst = cx.builder.fresh_temp("fnref", ty);
                    cx.builder.assign(cx.current, dst, rvalue);
                    MirOperand::Copy(dst)
                }
                None => MirOperand::Const(MirConstant::Unit),
            }
        }
        HirExprKind::FnTrampoline => {
            // A closure passed where a C `CFnPtr` is expected: emit the
            // address of a generated trampoline whose last argument is the
            // userdata pointer (the closure object) C threads back.
            match super::closure::lower_fn_trampoline(cx, &expr.ty, &expr.span) {
                Some(rvalue) => {
                    let dst = cx.builder.fresh_temp("trampoline", ty);
                    cx.builder.assign(cx.current, dst, rvalue);
                    MirOperand::Copy(dst)
                }
                None => MirOperand::Const(MirConstant::Unit),
            }
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
        HirExprKind::TypeName(arg) => lower_type_name(cx, arg, ty),
        HirExprKind::FieldNames(arg) => lower_field_names(cx, arg, ty),
        HirExprKind::FieldTypes(arg) => lower_field_types(cx, arg, ty),
        HirExprKind::VariantNames(arg) => lower_variant_names(cx, arg, ty),
        HirExprKind::VariantFieldTypes(arg) => lower_variant_field_types(cx, arg, ty),
        HirExprKind::PtrBuiltin { op, pointee, args } => {
            lower_ptr_builtin(cx, *op, pointee, args, ty)
        }
        HirExprKind::ReflectBuiltin { op, type_arg, args } => {
            lower_reflect_builtin(cx, *op, type_arg.as_ref(), args, ty)
        }
    }
}

/// Record runtime reflection metadata for `mty` (and transitively any
/// struct field types) into the function's accumulator, keyed by the
/// mangled type name. Idempotent: a type already recorded is skipped, so
/// the recursion over field types terminates on cycles.
fn record_reflect_type(cx: &mut LowerCx<'_>, mty: &MirType) {
    let key = mty.mangle();
    if cx.reflect_types.contains_key(&key) {
        return;
    }
    let (is_struct, fields) = match mty {
        MirType::Struct { name, .. } => {
            let decl = cx.decls.structs.get(name.as_str());
            let fields: Vec<ReflectField> = decl
                .map(|s| {
                    s.fields
                        .iter()
                        .map(|(fname, fty, _)| {
                            let fmty = mir_ty(fty, cx.subst);
                            ReflectField {
                                name: fname.clone(),
                                type_mangle: fmty.mangle(),
                                is_gc_ptr: crate::codegen::layout::is_gc_pointer(&fmty),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            (true, fields)
        }
        _ => (false, Vec::new()),
    };
    // Insert before recursing so a self-referential struct stops.
    cx.reflect_types.insert(
        key,
        ReflectType {
            name: format!("{}", mty),
            is_struct,
            fields: fields.clone(),
        },
    );
    if let MirType::Struct { name, .. } = mty {
        if let Some(decl) = cx.decls.structs.get(name.as_str()).copied() {
            for (_, fty, _) in &decl.fields {
                let fmty = mir_ty(fty, cx.subst);
                record_reflect_type(cx, &fmty);
            }
        }
    }
}

/// Lower a runtime reflection builtin. The boxed/target type argument is
/// grounded under the current substitution; reflection metadata for it is
/// recorded so the back end registers the runtime type record. See
/// `docs/v2/specs/runtime-reflection.md`.
fn lower_reflect_builtin(
    cx: &mut LowerCx<'_>,
    op: ReflectBuiltinOp,
    type_arg: Option<&crate::tycheck::Ty>,
    args: &[HirExpr],
    ty: MirType,
) -> MirOperand {
    let ops: Vec<MirOperand> = args.iter().map(|a| lower_expr(cx, a)).collect();
    match op {
        ReflectBuiltinOp::ToAny => {
            let value_ty = mir_ty(type_arg.expect("to_any carries a type argument"), cx.subst);
            record_reflect_type(cx, &value_ty);
            let dst = cx.builder.fresh_temp("any_box", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::AnyBox {
                    value: ops[0].clone(),
                    value_ty,
                },
            );
            MirOperand::Copy(dst)
        }
        ReflectBuiltinOp::Cast => {
            let target_ty = mir_ty(type_arg.expect("cast carries a type argument"), cx.subst);
            record_reflect_type(cx, &target_ty);
            let dst = cx.builder.fresh_temp("any_cast", ty.clone());
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::AnyCast {
                    any: ops[0].clone(),
                    target_ty,
                    option_ty: ty,
                },
            );
            MirOperand::Copy(dst)
        }
        ReflectBuiltinOp::TypeNameOf => {
            let dst = cx.builder.fresh_temp("type_name_of", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::AnyTypeName {
                    any: ops[0].clone(),
                },
            );
            MirOperand::Copy(dst)
        }
        ReflectBuiltinOp::FieldNamesOf => {
            let dst = cx.builder.fresh_temp("field_names_of", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::AnyFieldNames {
                    any: ops[0].clone(),
                },
            );
            MirOperand::Copy(dst)
        }
        ReflectBuiltinOp::GetField => {
            let dst = cx.builder.fresh_temp("get_field", ty.clone());
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::AnyGetField {
                    any: ops[0].clone(),
                    name: ops[1].clone(),
                    option_ty: ty,
                },
            );
            MirOperand::Copy(dst)
        }
        ReflectBuiltinOp::SetField => {
            let dst = cx.builder.fresh_temp("set_field", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::AnySetField {
                    any: ops[0].clone(),
                    name: ops[1].clone(),
                    value: ops[2].clone(),
                },
            );
            MirOperand::Copy(dst)
        }
    }
}

/// Lower `type_name<T>()`. The carried type argument is grounded under the
/// current substitution (so a generic parameter becomes the concrete type
/// for this monomorphization), then rendered to its source spelling as a
/// `String` constant. See `docs/v2/specs/reflection.md`.
fn lower_type_name(cx: &mut LowerCx<'_>, arg: &crate::tycheck::Ty, ty: MirType) -> MirOperand {
    let concrete = super::substitute(arg, cx.subst);
    let rendered = format!("{}", concrete.strip_self());
    let dst = cx.builder.fresh_temp("typename", ty);
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::Use(MirOperand::Const(MirConstant::Str(rendered))),
    );
    MirOperand::Copy(dst)
}

/// Lower `field_names<T>()`. The carried type argument is grounded under the
/// current substitution; when it is a known struct, the field names in
/// declaration order become a `List<String>` literal. A grounded non-struct
/// type yields an empty list (a generic parameter bound to a non-struct at
/// some instantiation cannot be rejected at the call site in this slice).
fn lower_field_names(cx: &mut LowerCx<'_>, arg: &crate::tycheck::Ty, ty: MirType) -> MirOperand {
    use crate::tycheck::Ty;
    let concrete = super::substitute(arg, cx.subst);
    let names: Vec<String> = match concrete.strip_self() {
        Ty::Struct { name, .. } => cx
            .decls
            .structs
            .get(name.as_str())
            .map(|s| s.fields.iter().map(|(n, _, _)| n.clone()).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let elements: Vec<MirOperand> = names
        .into_iter()
        .map(|n| MirOperand::Const(MirConstant::Str(n)))
        .collect();
    let list_ty = MirType::List(Box::new(MirType::Str));
    let dst = cx.builder.fresh_temp("fieldnames", ty);
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::ArrayLit {
            ty: list_ty,
            elements,
        },
    );
    MirOperand::Copy(dst)
}

/// Lower `field_types<T>()`. Mirrors `field_names`, but renders each field's
/// type name in declaration order instead of its name. For a generic struct
/// the declared field types carry the struct's own `Ty::Param`s; a
/// per-instantiation substitution built from the concrete type arguments
/// grounds them, so `field_types<Box<Int>>()` yields `["Int"]`. A grounded
/// non-struct type yields an empty list, matching `field_names`.
fn lower_field_types(cx: &mut LowerCx<'_>, arg: &crate::tycheck::Ty, ty: MirType) -> MirOperand {
    use crate::tycheck::Ty;
    let concrete = super::substitute(arg, cx.subst);
    let type_names: Vec<String> = match concrete.strip_self() {
        Ty::Struct { name, args, .. } => cx
            .decls
            .structs
            .get(name.as_str())
            .map(|s| {
                let mut subst: super::SubstMap = super::SubstMap::new();
                for (p, a) in struct_generic_params(s).iter().zip(args.iter()) {
                    subst.insert(p.clone(), a.clone());
                }
                s.fields
                    .iter()
                    .map(|(_, fty, _)| format!("{}", super::substitute(fty, &subst).strip_self()))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let elements: Vec<MirOperand> = type_names
        .into_iter()
        .map(|n| MirOperand::Const(MirConstant::Str(n)))
        .collect();
    let list_ty = MirType::List(Box::new(MirType::Str));
    let dst = cx.builder.fresh_temp("fieldtypes", ty);
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::ArrayLit {
            ty: list_ty,
            elements,
        },
    );
    MirOperand::Copy(dst)
}

/// Lower `variant_names<T>()`. The carried type argument is grounded under
/// the current substitution; when it is a known enum, the variant names in
/// declaration order become a `List<String>` literal. A grounded non-enum
/// type yields an empty list.
fn lower_variant_names(cx: &mut LowerCx<'_>, arg: &crate::tycheck::Ty, ty: MirType) -> MirOperand {
    use crate::tycheck::Ty;
    let concrete = super::substitute(arg, cx.subst);
    let names: Vec<String> = match concrete.strip_self() {
        Ty::Enum { name, .. } => cx
            .decls
            .enums
            .get(name.as_str())
            .map(|e| e.variants.iter().map(|v| v.name.clone()).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let elements: Vec<MirOperand> = names
        .into_iter()
        .map(|n| MirOperand::Const(MirConstant::Str(n)))
        .collect();
    let list_ty = MirType::List(Box::new(MirType::Str));
    let dst = cx.builder.fresh_temp("variantnames", ty);
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::ArrayLit {
            ty: list_ty,
            elements,
        },
    );
    MirOperand::Copy(dst)
}

/// Lower `variant_field_types<T>()` to a `List<List<String>>`: one inner list
/// per variant (declaration order) of that variant's payload field type
/// names, empty for a unit variant. Each declared field type is grounded
/// under a per-instantiation substitution from the enum's generic parameters
/// to the concrete type arguments, so a generic payload renders its concrete
/// type. A grounded non-enum type yields an empty outer list.
fn lower_variant_field_types(
    cx: &mut LowerCx<'_>,
    arg: &crate::tycheck::Ty,
    ty: MirType,
) -> MirOperand {
    use crate::tycheck::Ty;
    let str_list_ty = MirType::List(Box::new(MirType::Str));
    let concrete = super::substitute(arg, cx.subst);
    // Render each variant's payload types into a vector of strings, in
    // declaration order, before touching the builder.
    let per_variant: Vec<Vec<String>> = match concrete.strip_self() {
        Ty::Enum { name, args, .. } => cx
            .decls
            .enums
            .get(name.as_str())
            .map(|e| {
                let mut subst: super::SubstMap = super::SubstMap::new();
                for (p, a) in enum_generic_params(e).iter().zip(args.iter()) {
                    subst.insert(p.clone(), a.clone());
                }
                e.variants
                    .iter()
                    .map(|v| {
                        v.fields
                            .iter()
                            .map(|(_, fty, _)| {
                                format!("{}", super::substitute(fty, &subst).strip_self())
                            })
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    // Build each inner `List<String>` into its own temp, then collect those
    // temps as the outer list's elements.
    let mut outer: Vec<MirOperand> = Vec::with_capacity(per_variant.len());
    for variant in per_variant {
        let inner_elements: Vec<MirOperand> = variant
            .into_iter()
            .map(|n| MirOperand::Const(MirConstant::Str(n)))
            .collect();
        let inner = cx.builder.fresh_temp("variantfields", str_list_ty.clone());
        cx.builder.assign(
            cx.current,
            inner,
            MirRvalue::ArrayLit {
                ty: str_list_ty.clone(),
                elements: inner_elements,
            },
        );
        outer.push(MirOperand::Copy(inner));
    }
    let dst = cx.builder.fresh_temp("variantfieldtypes", ty);
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::ArrayLit {
            ty: MirType::List(Box::new(str_list_ty)),
            elements: outer,
        },
    );
    MirOperand::Copy(dst)
}

/// Lower a raw-pointer FFI builtin. The pointee is grounded under the
/// current substitution so its machine width is known; the resulting MIR
/// rvalue or statement carries it to codegen. See `docs/v2/specs/std-ffi.md`.
fn lower_ptr_builtin(
    cx: &mut LowerCx<'_>,
    op: PtrBuiltinOp,
    pointee: &crate::tycheck::Ty,
    args: &[HirExpr],
    ty: MirType,
) -> MirOperand {
    let pointee = mir_ty(pointee, cx.subst);
    let mut ops: Vec<MirOperand> = args.iter().map(|a| lower_expr(cx, a)).collect();
    match op {
        PtrBuiltinOp::Null => {
            let dst = cx.builder.fresh_temp("ptr_null", ty);
            cx.builder.assign(cx.current, dst, MirRvalue::PtrNull);
            MirOperand::Copy(dst)
        }
        PtrBuiltinOp::Alloc => {
            let count = ops.remove(0);
            let dst = cx.builder.fresh_temp("ptr_alloc", ty);
            cx.builder
                .assign(cx.current, dst, MirRvalue::PtrAlloc { count, pointee });
            MirOperand::Copy(dst)
        }
        PtrBuiltinOp::Load => {
            let addr = ops.remove(0);
            let dst = cx.builder.fresh_temp("ptr_load", ty);
            cx.builder
                .assign(cx.current, dst, MirRvalue::PtrLoad { addr, pointee });
            MirOperand::Copy(dst)
        }
        PtrBuiltinOp::Offset => {
            let addr = ops.remove(0);
            let count = ops.remove(0);
            let dst = cx.builder.fresh_temp("ptr_offset", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::PtrOffset {
                    addr,
                    count,
                    pointee,
                },
            );
            MirOperand::Copy(dst)
        }
        PtrBuiltinOp::IsNull => {
            let addr = ops.remove(0);
            let dst = cx.builder.fresh_temp("ptr_is_null", ty);
            cx.builder
                .assign(cx.current, dst, MirRvalue::PtrIsNull { addr });
            MirOperand::Copy(dst)
        }
        PtrBuiltinOp::Store => {
            let addr = ops.remove(0);
            let value = ops.remove(0);
            cx.builder.emit(
                cx.current,
                MirStatement::PtrStore {
                    addr,
                    value,
                    pointee,
                },
            );
            MirOperand::Const(MirConstant::Unit)
        }
        PtrBuiltinOp::Free => {
            let addr = ops.remove(0);
            cx.builder.emit(cx.current, MirStatement::PtrFree { addr });
            MirOperand::Const(MirConstant::Unit)
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

/// Lower `lhs && rhs` (`is_and`) or `lhs || rhs` with short-circuit semantics.
/// The left operand is evaluated first; the right operand runs only when the
/// left does not settle the answer (left true for `&&`, left false for `||`).
/// Otherwise the result is the short-circuit constant (`false` for `&&`,
/// `true` for `||`) and the right operand is never touched.
fn lower_short_circuit(
    cx: &mut LowerCx<'_>,
    is_and: bool,
    lhs: &HirExpr,
    rhs: &HirExpr,
    ty: MirType,
) -> MirOperand {
    let discr = lower_expr(cx, lhs);
    let rhs_bb = cx.builder.new_block();
    let short_bb = cx.builder.new_block();
    let cont_bb = cx.builder.new_block();
    let result = cx.builder.fresh_temp(if is_and { "and" } else { "or" }, ty);

    // For `&&`, a true left falls through to the right operand and a false left
    // takes the short-circuit block. For `||` it is the other way round.
    let (then_target, else_target) = if is_and {
        (rhs_bb, short_bb)
    } else {
        (short_bb, rhs_bb)
    };
    cx.builder.close_block(
        cx.current,
        MirTerminator::SwitchInt {
            discriminant: discr,
            targets: vec![(1, then_target)],
            otherwise: else_target,
        },
    );

    cx.current = rhs_bb;
    let rv = lower_expr(cx, rhs);
    cx.builder.assign(cx.current, result, MirRvalue::Use(rv));
    if !cx.builder.is_closed(cx.current) {
        cx.builder
            .close_block(cx.current, MirTerminator::Goto(cont_bb));
    }

    cx.current = short_bb;
    cx.builder.assign(
        cx.current,
        result,
        MirRvalue::Use(MirOperand::Const(MirConstant::Bool(!is_and))),
    );
    if !cx.builder.is_closed(cx.current) {
        cx.builder
            .close_block(cx.current, MirTerminator::Goto(cont_bb));
    }

    cx.current = cont_bb;
    // The merge block is reachable even if the right operand diverged.
    cx.diverged = false;
    MirOperand::Copy(result)
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
    cx.builder.mark_loop_header(header);
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
    cx.builder.mark_loop_header(header);
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

/// Lower a `String` `==`/`!=` into a call to the runtime byte-equality
/// intrinsic. `==` yields the call result directly; `!=` negates it.
fn lower_string_eq(
    cx: &mut LowerCx<'_>,
    op: HirBinaryOp,
    lhs: &HirExpr,
    rhs: &HirExpr,
    ty: MirType,
) -> MirOperand {
    let l = lower_expr(cx, lhs);
    let r = lower_expr(cx, rhs);
    let eq = cx.builder.fresh_temp("streq", MirType::Bool);
    cx.builder.assign(
        cx.current,
        eq,
        MirRvalue::Call {
            callee: MirFnRef {
                mangled: super::super::intrinsics::STR_EQ.into(),
                origin: None,
            },
            args: vec![l, r],
        },
    );
    match op {
        HirBinaryOp::Eq => MirOperand::Copy(eq),
        _ => {
            let dst = cx.builder.fresh_temp("strne", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::UnaryOp(MirUnOp::Not, MirOperand::Copy(eq)),
            );
            MirOperand::Copy(dst)
        }
    }
}

/// Lower an ordering comparison (`< <= > >=`) on two `String` operands.
/// Calls the `raven_string_cmp` intrinsic, which returns a negative, zero,
/// or positive `Int`, then applies the operator against `0`.
fn lower_string_cmp(
    cx: &mut LowerCx<'_>,
    op: HirBinaryOp,
    lhs: &HirExpr,
    rhs: &HirExpr,
    ty: MirType,
) -> MirOperand {
    let l = lower_expr(cx, lhs);
    let r = lower_expr(cx, rhs);
    let cmp = cx.builder.fresh_temp("strcmp", MirType::Int);
    cx.builder.assign(
        cx.current,
        cmp,
        MirRvalue::Call {
            callee: MirFnRef {
                mangled: super::super::intrinsics::STR_CMP.into(),
                origin: None,
            },
            args: vec![l, r],
        },
    );
    let dst = cx.builder.fresh_temp("strord", ty);
    cx.builder.assign(
        cx.current,
        dst,
        MirRvalue::BinaryOp(
            map_binary(op),
            MirOperand::Copy(cmp),
            MirOperand::Const(MirConstant::Int(0)),
        ),
    );
    MirOperand::Copy(dst)
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
    // Build the per-part String operands first. Each part is bound to a
    // temp local so it lives in a GC-rooted slot: the fold calls
    // `raven_string_concat`, which allocates internally and can trigger a
    // collection, and an unrooted literal-text part (a bare `Const::Str`
    // codegen promotes to a heap String at the call site) would be freed
    // mid-concat. Binding every part, text and expression alike, keeps it
    // reachable across the concat chain.
    let mut operands: Vec<MirOperand> = Vec::with_capacity(parts.len());
    for p in parts {
        match p {
            InterpolPart::Text(s) => {
                let dst = cx.builder.fresh_temp("interp_text", MirType::Str);
                cx.builder.assign(
                    cx.current,
                    dst,
                    MirRvalue::Use(MirOperand::Const(MirConstant::Str(s.clone()))),
                );
                operands.push(MirOperand::Copy(dst));
            }
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
/// through their `to_string` runtime intrinsic. Any non-scalar part
/// (a generic `T: ToString` or a user type with a `ToString` impl) was
/// already rewritten into a `to_string()` method call during HIR
/// lowering, so it arrives here typed `String` and takes the as-is path;
/// the catch-all is unreachable in practice and uses the value directly.
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
///
/// A generic struct (`struct Box<T> { value: T }`) is instantiated here:
/// the literal's resolved type carries the concrete type arguments, which
/// are matched against the struct declaration's own generic parameters to
/// build a per-instantiation substitution. Each declared field type is
/// then substituted to a ground type, so a `Box<Int>` lays out an `Int`
/// slot and a `Box<String>` lays out a traced `String` slot. The
/// per-instantiation `MirType::Struct` carries the concrete arguments, so
/// the back end mangles each instantiation to a distinct descriptor with
/// the right GC pointer mask.
fn lower_struct_lit(
    cx: &mut LowerCx<'_>,
    name: &str,
    fields: &[(String, HirExpr)],
    expr_ty: &crate::tycheck::Ty,
    ty: MirType,
) -> MirOperand {
    // Build the per-instantiation field substitution from the literal's
    // resolved type. The struct's declared field types carry the struct's
    // own `Ty::Param`s; matching the declaration's generic parameters
    // against the concrete type arguments at this site binds each to a
    // ground type. The literal's type has the enclosing function's
    // substitution applied first so a caller's own `Ty::Param` is already
    // ground.
    let struct_subst = cx.decls.structs.get(name).map(|s| {
        let mut subst: super::SubstMap = super::SubstMap::new();
        let params = struct_generic_params(s);
        if let crate::tycheck::Ty::Struct { args, .. } = super::substitute(expr_ty, cx.subst) {
            for (p, arg) in params.iter().zip(args.iter()) {
                subst.insert(p.clone(), arg.clone());
            }
        }
        subst
    });

    // Declaration order from the struct table, when available. Each field
    // type is substituted under the per-instantiation struct substitution
    // (a generic field `T` becomes its concrete type) and then under the
    // enclosing function's substitution so any remaining parameter is
    // ground.
    let decl_order: Option<Vec<(String, MirType)>> = cx.decls.structs.get(name).map(|s| {
        let struct_subst = struct_subst.as_ref();
        s.fields
            .iter()
            .map(|(fname, fty, _)| {
                let ground = match struct_subst {
                    Some(sub) => super::substitute(fty, sub),
                    None => fty.clone(),
                };
                (fname.clone(), mir_ty(&ground, cx.subst))
            })
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
fn call_ref_from_callee(
    cx: &mut LowerCx<'_>,
    callee: &HirExpr,
    args: &[HirExpr],
    type_args: &[crate::tycheck::Ty],
    result_ty: &crate::tycheck::Ty,
) -> MirFnRef {
    let (HirExprKind::Ident(name) | HirExprKind::FnRef(name)) = &callee.kind else {
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
    // Explicit type arguments bind the callee's generic parameters in
    // declaration order. Each is grounded under the enclosing function's
    // substitution first, so a `T` argument inside a generic body resolves
    // to the concrete type for this monomorphization. This is what lets a
    // call carrying `T` only as a type argument (no value argument pins it)
    // specialize correctly, for example `type_name<T>()` and `describe<T>()`.
    for (param, arg) in entry.generic_params.iter().zip(type_args.iter()) {
        let got = super::substitute(arg, cx.subst);
        subst.entry(param.clone()).or_insert(got);
    }
    for (decl_ty, arg) in entry.params.iter().zip(args.iter()) {
        let got = super::substitute(&arg.ty, cx.subst);
        match_param(decl_ty, &got, &mut subst);
    }
    // Also match the declared return type against the call's resolved
    // result type. A generic parameter that appears only in the return
    // type or in a bound (for example `T` in
    // `collect<T, S: Iterator<T>>(it: S) -> List<T>`, where `T` is never a
    // parameter) cannot be bound from the arguments alone. The type
    // checker already inferred it (the call site's result type is
    // concrete, `List<Int>`), so matching `List<T>` against it recovers
    // the binding the monomorphizer needs.
    let concrete_result = super::substitute(result_ty, cx.subst);
    match_param(&entry.ret, &concrete_result, &mut subst);
    let mangled = super::mono_symbol(name, &entry.generic_params, &subst);
    cx.pending_calls.push((entry.decl, subst));
    MirFnRef {
        mangled,
        origin: None,
    }
}

/// Recover a struct declaration's generic parameters in declaration
/// order. The HIR struct carries no explicit parameter list, so the
/// parameters are recovered by scanning the field types for `Ty::Param`
/// occurrences and ordering them by their declaration index. The order
/// fixes how the literal's concrete type arguments map onto the
/// parameters, matching how the type checker built the argument list.
pub(super) fn struct_generic_params(s: &crate::hir::HirStruct) -> Vec<crate::tycheck::ty::ParamId> {
    let mut found: Vec<crate::tycheck::ty::ParamId> = Vec::new();
    for (_, ty, _) in &s.fields {
        collect_ty_params(ty, &mut found);
    }
    found.sort_by_key(|p| p.index);
    found.dedup();
    found
}

/// The generic parameters of an enum declaration, in `ParamId` order. Like
/// `struct_generic_params` but gathered across every variant's payload field
/// types, so an enum's instantiation can ground its variant payloads.
fn enum_generic_params(e: &crate::hir::HirEnum) -> Vec<crate::tycheck::ty::ParamId> {
    let mut found: Vec<crate::tycheck::ty::ParamId> = Vec::new();
    for v in &e.variants {
        for (_, ty, _) in &v.fields {
            collect_ty_params(ty, &mut found);
        }
    }
    found.sort_by_key(|p| p.index);
    found.dedup();
    found
}

/// Walk a type and push every distinct `Ty::Param` into `out`.
fn collect_ty_params(t: &crate::tycheck::Ty, out: &mut Vec<crate::tycheck::ty::ParamId>) {
    use crate::tycheck::Ty;
    match t {
        Ty::Param(p) => {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        Ty::Option(t) | Ty::List(t) | Ty::SelfTy(t) => collect_ty_params(t, out),
        Ty::Result(a, b) => {
            collect_ty_params(a, out);
            collect_ty_params(b, out);
        }
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => {
            for a in args {
                collect_ty_params(a, out);
            }
        }
        Ty::Function { params, ret } => {
            for p in params {
                collect_ty_params(p, out);
            }
            collect_ty_params(ret, out);
        }
        _ => {}
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
        // A resolved function reference (the callee the HIR bound to a
        // top-level function) is always a direct call by symbol, even when a
        // call-site local of the same spelling is in scope.
        HirExprKind::FnRef(_) => false,
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
    result_ty: &crate::tycheck::Ty,
) -> MirOperand {
    let recv_ty = mir_ty(&receiver.ty, cx.subst);

    // The integer C FFI types have no `ToString` impl of their own. A
    // `to_string` call on one (inserted by HIR lowering for a `print`
    // argument or an interpolation fragment) widens the value to `Int`
    // and renders through the `Int` to-string runtime intrinsic. The
    // widening is a sign extension (`CInt` is i32; `CLong`/`CSize` are
    // already pointer-width and pass through). `CSize` is treated as a
    // signed `Int`, correct for realistic sizes (below 2^63).
    if name == "to_string" {
        if let MirType::Ffi(MirFfiTy::CInt | MirFfiTy::CLong | MirFfiTy::CSize) = &recv_ty {
            let recv = lower_expr(cx, receiver);
            let widened = cx.builder.fresh_temp("ffiwiden", MirType::Int);
            cx.builder.assign(
                cx.current,
                widened,
                MirRvalue::Cast {
                    operand: recv,
                    target: MirType::Int,
                },
            );
            let dst = cx.builder.fresh_temp("tostr", MirType::Str);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::Call {
                    callee: MirFnRef {
                        mangled: super::super::intrinsics::INT_TO_STRING.into(),
                        origin: None,
                    },
                    args: vec![MirOperand::Copy(widened)],
                },
            );
            return MirOperand::Copy(dst);
        }
        // The float C FFI types render through the `Float` to-string path.
        // A `CFloat` (f32) widens to f64 first; a `CDouble` is already f64.
        if let MirType::Ffi(MirFfiTy::CFloat | MirFfiTy::CDouble) = &recv_ty {
            let recv = lower_expr(cx, receiver);
            let widened = cx.builder.fresh_temp("ffiwiden", MirType::Float);
            cx.builder.assign(
                cx.current,
                widened,
                MirRvalue::Cast {
                    operand: recv,
                    target: MirType::Float,
                },
            );
            let dst = cx.builder.fresh_temp("tostr", MirType::Str);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::Call {
                    callee: MirFnRef {
                        mangled: super::super::intrinsics::FLOAT_TO_STRING.into(),
                        origin: None,
                    },
                    args: vec![MirOperand::Copy(widened)],
                },
            );
            return MirOperand::Copy(dst);
        }
    }

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

    // A built-in `List<T>` method has no user `impl`; route it to a
    // dedicated rvalue carrying the element type so the back end can size
    // slots and pick the GC-pointer flag. Any user `impl` on a list type
    // would have been resolved away before reaching the built-in set, so
    // recognizing the fixed method names here is safe.
    if let MirType::List(elem) = &recv_ty {
        if let Some(op) = list_method_op(name) {
            let elem_ty = (**elem).clone();
            let recv = lower_expr(cx, receiver);
            let arg = args.first().map(|a| lower_expr(cx, a));
            let dst = cx.builder.fresh_temp("lmethod", ty);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::ListMethod {
                    op,
                    receiver: recv,
                    arg,
                    elem_ty,
                },
            );
            return MirOperand::Copy(dst);
        }
    }

    // Built-in `String` methods `len`/`is_empty` have no definition symbol
    // unless the program writes an `impl String`. Route them to the length
    // intrinsic when unshadowed, mirroring the `List` built-ins, so `.len()`
    // works without an import and matches `List`/`Map`/`Set`. A user or
    // stdlib `impl String` of the same name takes precedence and is reached
    // through static dispatch below.
    if recv_ty == MirType::Str && !str_has_user_method(cx, name) {
        if name == "len" {
            let recv = lower_expr(cx, receiver);
            let dst = cx.builder.fresh_temp("strlen", MirType::Int);
            cx.builder.assign(cx.current, dst, str_len_call(recv));
            return MirOperand::Copy(dst);
        }
        if name == "is_empty" {
            let recv = lower_expr(cx, receiver);
            let len = cx.builder.fresh_temp("strlen", MirType::Int);
            cx.builder.assign(cx.current, len, str_len_call(recv));
            let dst = cx.builder.fresh_temp("strempty", MirType::Bool);
            cx.builder.assign(
                cx.current,
                dst,
                MirRvalue::BinaryOp(
                    MirBinOp::Eq,
                    MirOperand::Copy(len),
                    MirOperand::Const(MirConstant::Int(0)),
                ),
            );
            return MirOperand::Copy(dst);
        }
    }

    // Built-in numeric conversions `Int.to_float()` and `Float.to_int()` have
    // no definition symbol; lower them to a scalar cast (an `fcvt`), unless a
    // user `impl` shadows the name, mirroring the `String` built-ins above.
    // `to_int` truncates toward zero.
    if recv_ty == MirType::Int
        && name == "to_float"
        && !prim_has_user_method(cx, name, &MirType::Int)
    {
        let recv = lower_expr(cx, receiver);
        let dst = cx.builder.fresh_temp("tofloat", MirType::Float);
        cx.builder.assign(
            cx.current,
            dst,
            MirRvalue::Cast {
                operand: recv,
                target: MirType::Float,
            },
        );
        return MirOperand::Copy(dst);
    }
    if recv_ty == MirType::Float
        && name == "to_int"
        && !prim_has_user_method(cx, name, &MirType::Float)
    {
        let recv = lower_expr(cx, receiver);
        let dst = cx.builder.fresh_temp("toint", MirType::Int);
        cx.builder.assign(
            cx.current,
            dst,
            MirRvalue::Cast {
                operand: recv,
                target: MirType::Int,
            },
        );
        return MirOperand::Copy(dst);
    }

    // Static dispatch: build the per-type method symbol from the receiver
    // type so it matches the impl method's definition symbol. When the
    // method is generic (a method on `impl<T> Box<T>`, whose declared
    // types carry `Ty::Param`), queue its instantiation so the worklist
    // emits the body specialized for this receiver. The symbol is the same
    // `<RecvType>$<method>` either way: the worklist recomputes it from the
    // concrete implementing type, which the receiver type fixes here.
    // A generic method's per-instantiation symbol carries any method-level
    // type arguments as a suffix (`Box_Int$mapped$Bool`), so the call site
    // uses the symbol the queuing step computes rather than the bare
    // `<RecvType>$<method>` name, which would collide across method-level
    // instantiations. A concrete-receiver method or one with no method-level
    // parameters keeps the bare per-type symbol.
    let symbol = queue_generic_method(cx, &receiver.ty, name, args, result_ty)
        .unwrap_or_else(|| recv_ty.method_symbol(name));
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

/// Lower an associated function call `Type.func(args)`. Like a static
/// method call but with no receiver argument. The per-type symbol comes
/// from the named implementing type; a generic type's instantiation is
/// queued the same way a generic method's is.
fn lower_assoc_call(
    cx: &mut LowerCx<'_>,
    self_ty: &crate::tycheck::Ty,
    name: &str,
    args: &[HirExpr],
    ty: MirType,
    result_ty: &crate::tycheck::Ty,
) -> MirOperand {
    let recv_ty = mir_ty(self_ty, cx.subst);
    let symbol = queue_generic_method(cx, self_ty, name, args, result_ty)
        .unwrap_or_else(|| recv_ty.method_symbol(name));
    let arg_ops: Vec<MirOperand> = args.iter().map(|a| lower_expr(cx, a)).collect();
    let dst = cx.builder.fresh_temp("acall", ty);
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

/// Queue a generic method's instantiation for the monomorphizer.
///
/// A method on a generic implementing type (`impl<T> Box<T> { fun
/// unwrap(self) -> T }`) carries `Ty::Param`s in its declared `self` and
/// return types, so it must be specialized for each concrete receiver,
/// the same way a generic free function is specialized at a call site.
/// The concrete receiver type fixes the implementing type's arguments;
/// matching the declared implementing type against it (and each declared
/// parameter type against the concrete argument type) builds the
/// substitution. The worklist recomputes the symbol from the substituted
/// implementing type, so no symbol is returned here. A concrete-receiver
/// method is already a monomorphization root and needs no queuing.
fn queue_generic_method(
    cx: &mut LowerCx<'_>,
    receiver_ty: &crate::tycheck::Ty,
    name: &str,
    args: &[HirExpr],
    result_ty: &crate::tycheck::Ty,
) -> Option<String> {
    let entries = cx.decls.methods.get(name).cloned()?;
    let recv = super::substitute(receiver_ty, cx.subst);
    let recv = recv.strip_self().clone();
    let concrete_result = super::substitute(result_ty, cx.subst);
    // Pick the generic method whose implementing type matches the concrete
    // receiver. A concrete-receiver method is skipped: it is reached
    // through its own root. The structural match against the implementing
    // type both selects the entry and binds the impl's parameters.
    for entry in entries.iter().filter(|e| e.generic) {
        let mut subst: super::SubstMap = super::SubstMap::new();
        match_param(&entry.self_ty, &recv, &mut subst);
        // Also bind any method-level parameters from the concrete argument
        // types. A method's own `<U>` is grounded by matching its declared
        // parameter types (which carry `U`) against the concrete argument
        // types, so the substitution carries both the impl's `T` and the
        // method's `U`.
        for (decl_ty, arg) in entry.params.iter().zip(args.iter()) {
            let got = super::substitute(&arg.ty, cx.subst);
            match_param(decl_ty, &got, &mut subst);
        }
        // Bind any method-level parameter that appears only in the return
        // type (`fun decode<T>() -> Result<T, E>`) by matching the declared
        // return against the call's resolved result type. The self and
        // parameter types cannot ground it; the type checker already
        // inferred it, and this recovers it for monomorphization so the
        // method body specializes correctly instead of defaulting to `Unit`.
        match_param(&entry.ret, &concrete_result, &mut subst);
        // The match must have grounded the implementing type to the
        // receiver; if it did not (a non-matching impl of the same method
        // name on another type), skip this entry.
        let concrete_self = super::substitute(&entry.self_ty, &subst);
        if MirType::from_ty(&concrete_self) == MirType::from_ty(&recv) {
            // The instantiation symbol the worklist will emit: the concrete
            // implementing type's mangle plus `$<method>`, with each
            // method-level type argument appended. The call site references
            // this exact symbol so it resolves to the instantiation lowered
            // for these receiver and method-level type arguments.
            let concrete_self_mangle = MirType::from_ty(&concrete_self).mangle();
            let symbol = super::method_mono_symbol(
                &concrete_self_mangle,
                name,
                &entry.method_params,
                &subst,
            );
            cx.pending_calls.push((entry.decl, subst));
            return Some(symbol);
        }
    }
    None
}

/// Resolve a field's slot index from the receiver's struct type and the
/// field name. The index is the field's position in declaration order,
/// which matches the slot offset the struct constructor writes. Falls
/// back to `0` only when the receiver is not a known struct (which a
/// checked program never produces).
pub(super) fn field_index_from_ty(ty: &crate::tycheck::Ty, cx: &LowerCx<'_>, name: &str) -> usize {
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

/// Map a built-in `List<T>` method name to its [`ListMethodOp`], or
/// `None` when the name is not one of the recognized list methods. The
/// set mirrors `tycheck::builtin::list_methods`.
fn list_method_op(name: &str) -> Option<ListMethodOp> {
    Some(match name {
        "len" => ListMethodOp::Len,
        "is_empty" => ListMethodOp::IsEmpty,
        "push" => ListMethodOp::Push,
        "pop" => ListMethodOp::Pop,
        "get" => ListMethodOp::Get,
        _ => return None,
    })
}

/// A call to the `__str_len` runtime intrinsic on `recv`, returning `Int`.
fn str_len_call(recv: MirOperand) -> MirRvalue {
    MirRvalue::Call {
        callee: MirFnRef {
            mangled: super::super::intrinsics::STR_LEN.into(),
            origin: None,
        },
        args: vec![recv],
    }
}

/// True when the program defines a `String` method named `name` (a user or
/// stdlib `impl String`). Such a method shadows the built-in `len`/`is_empty`
/// fast path and is reached through static dispatch instead.
fn str_has_user_method(cx: &LowerCx<'_>, name: &str) -> bool {
    prim_has_user_method(cx, name, &MirType::Str)
}

/// Whether the program defines an `impl` method `name` whose receiver is the
/// primitive type `ty`, so a built-in of the same name should defer to it.
fn prim_has_user_method(cx: &LowerCx<'_>, name: &str, ty: &MirType) -> bool {
    cx.decls
        .methods
        .get(name)
        .is_some_and(|entries| entries.iter().any(|e| MirType::from_ty(&e.self_ty) == *ty))
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
