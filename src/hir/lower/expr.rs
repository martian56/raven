//! Lowering AST expressions and blocks to HIR.

use crate::ast::{BinaryOp, Block as AstBlock, ElseBranch, Expr, ExprKind, LambdaBody, UnaryOp};
use crate::error::RavenError;
use crate::span::Span;
use crate::tycheck::Ty;

use crate::hir::expr::{
    HirArm, HirBinaryOp, HirBlock, HirExpr, HirExprKind, HirUnaryOp, InterpolPart, PtrBuiltinOp,
    ReflectBuiltinOp,
};
use crate::hir::pattern::{HirPattern, HirPatternKind};
use crate::hir::stmt::{HirAssignTarget, HirStmt, HirStmtKind};

use super::pattern::lower_pattern;
use super::stmt::lower_stmt;
use super::sugar::{assign_stmt, block_of_tail, ident_expr, let_stmt, make_expr};
use super::LowerCtx;

/// Lower a block. The `expected_ty` is used as a hint for tail-less
/// blocks; an empty block evaluates to `Unit`.
pub(crate) fn lower_block_to_block(
    block: &AstBlock,
    expected_ty: &Ty,
    cx: &LowerCtx<'_>,
) -> Result<HirBlock, RavenError> {
    let mut stmts = Vec::with_capacity(block.stmts.len());
    for s in &block.stmts {
        stmts.extend(lower_stmt(s, cx)?);
    }
    let (tail, ty) = match &block.trailing {
        Some(e) => {
            let lowered = lower_expr(e, expected_ty, cx)?;
            let ty = lowered.ty.clone();
            (Some(Box::new(lowered)), ty)
        }
        None => (None, Ty::Unit),
    };
    Ok(HirBlock {
        stmts,
        tail,
        ty,
        span: block.span.clone(),
    })
}

/// Lower a single expression body into a block whose tail is that
/// expression. Used for `fun add(...) -> Int = a + b` bodies.
pub(crate) fn lower_expr_as_block(
    e: &Expr,
    expected_ty: &Ty,
    cx: &LowerCtx<'_>,
) -> Result<HirBlock, RavenError> {
    let lowered = lower_expr(e, expected_ty, cx)?;
    Ok(block_of_tail(lowered))
}

/// Lower one expression. `_hint` is currently unused but reserved for
/// future hints (e.g. `if`-as-expression target type).
pub(crate) fn lower_expr(
    expr: &Expr,
    _hint: &Ty,
    cx: &LowerCtx<'_>,
) -> Result<HirExpr, RavenError> {
    let ty = cx.ty_at(&expr.span);
    let span = expr.span.clone();
    // A closure passed where a `CFnPtr` is expected lowers to a trampoline
    // address. The closure expression is not evaluated here; the closure
    // object is passed separately to the C function's userdata parameter.
    if cx.is_closure_callback(&span) {
        return Ok(make_expr(HirExprKind::FnTrampoline, ty, span));
    }
    let kind = match &expr.kind {
        // A macro call should have been expanded before HIR lowering; reaching
        // here means the token pre-pass was skipped (only the formatter parses
        // un-expanded source, and it never lowers to HIR).
        ExprKind::MacroCall(_) => {
            return Err(super::ty_error(
                "internal error: macro call reached HIR lowering without expansion",
                &span,
            ))
        }
        ExprKind::Int(i) => HirExprKind::Int(*i),
        ExprKind::Float(f) => HirExprKind::Float(*f),
        ExprKind::Bool(b) => HirExprKind::Bool(*b),
        ExprKind::Str(s) | ExprKind::BlockStr(s) => HirExprKind::Str(s.clone()),
        ExprKind::InterpolatedString(fragments) => {
            let mut parts = Vec::with_capacity(fragments.len());
            for frag in fragments {
                match frag {
                    crate::ast::StrFragment::Literal(text) => {
                        parts.push(InterpolPart::Text(text.clone()))
                    }
                    crate::ast::StrFragment::Expr(e) => {
                        // Each fragment was parsed as a real expression and
                        // type-checked under its own span, so it lowers like
                        // any other value. The built-in scalars (and a
                        // `String`) lower to MIR per-type to-string
                        // conversions and a concat. Any other type is
                        // routed through its `ToString` impl here, so the
                        // MIR part is already a `String`.
                        let lowered = lower_expr(e, &Ty::Str, cx)?;
                        parts.push(InterpolPart::Expr(to_string_if_needed(lowered)));
                    }
                }
            }
            HirExprKind::Interpolate(parts)
        }
        ExprKind::CStr(s) => HirExprKind::CStr(s.clone()),
        ExprKind::Char(c) => HirExprKind::Char(*c),
        ExprKind::SelfLower => HirExprKind::SelfValue,
        ExprKind::SelfUpper => HirExprKind::Ident("Self".into()),
        ExprKind::Ident { name, .. } if name == "None" => HirExprKind::NoneCtor,
        ExprKind::Ident { name, .. } => {
            // A reference to a mutable module-level `let` reads its global
            // data slot.
            if let Some(global) = module_global_name(&expr.span, cx) {
                return Ok(make_expr(HirExprKind::GlobalGet(global), ty, span));
            }
            // A reference to a module-level `const` inlines its folded
            // constant value (`60 * 60` becomes the literal `3600`).
            if let Some(kind) = module_const_literal(&expr.span, cx) {
                return Ok(make_expr(kind, ty, span));
            }
            // A use that binds to a top level function carries that
            // function's declared name, which may differ from the source
            // spelling when the function was namespaced (a bundled stdlib
            // function such as `std.io.println`). Any other identifier
            // keeps its source spelling.
            let fn_name = cx.fn_name_at(&expr.span);
            // A top-level function used as a value (its type is a function
            // type; a callee is handled by the call lowering, not here)
            // becomes a closure object, so a named function has the same
            // representation as a lambda and can be passed to a higher-order
            // function. The exception is a C-FFI callback the type checker
            // recorded, which keeps the raw function-address form.
            match fn_name {
                Some(n) if matches!(ty, Ty::Function { .. }) && !cx.is_callback_fn(&expr.span) => {
                    HirExprKind::FnClosure { name: n }
                }
                Some(n) => HirExprKind::Ident(n),
                None => HirExprKind::Ident(name.clone()),
            }
        }
        ExprKind::Array(items) => {
            let elem_hint = match &ty {
                Ty::List(t) => (**t).clone(),
                _ => Ty::Error,
            };
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(lower_expr(it, &elem_hint, cx)?);
            }
            HirExprKind::Array(out)
        }
        ExprKind::SetLit(items) => return lower_set_lit(items, &ty, &span, cx),
        ExprKind::MapLit(pairs) => return lower_map_lit(pairs, &ty, &span, cx),
        ExprKind::Tuple(_) => {
            // The parser produces tuples but the resolver/tycheck
            // reject them; this branch should not run in practice.
            return Err(super::ty_error(
                "tuple expressions are not supported",
                &span,
            ));
        }
        ExprKind::Paren(inner) => {
            let lowered = lower_expr(inner, &ty, cx)?;
            HirExprKind::Paren(Box::new(lowered))
        }
        ExprKind::Block(b) => HirExprKind::Block(lower_block_to_block(b, &ty, cx)?),
        ExprKind::Unary { op, operand } => {
            let lowered = lower_expr(operand, &Ty::Error, cx)?;
            HirExprKind::Unary {
                op: lower_unop(*op),
                operand: Box::new(lowered),
            }
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let l = lower_expr(lhs, &Ty::Error, cx)?;
            let r = lower_expr(rhs, &Ty::Error, cx)?;
            // `==`/`!=` on a struct or enum that implements `Eq` (a derived or
            // hand-written `equals` method) dispatch to that method, so
            // equality is by value rather than object identity. The
            // conversion is a `MethodCall`, which MIR resolves to the type's
            // `equals` symbol (the same way `print` routes through
            // `to_string`). `!=` negates the result. A primitive keeps the
            // native compare below, and `String` keeps its own contents path
            // in MIR; a type without an `Eq` impl is unchanged.
            if matches!(op, BinaryOp::Eq | BinaryOp::Ne)
                && type_has_equals(cx.env, l.ty.strip_self())
            {
                let call = HirExprKind::MethodCall {
                    receiver: Box::new(l),
                    name: "equals".into(),
                    args: vec![r],
                };
                if matches!(op, BinaryOp::Ne) {
                    let eq = make_expr(call, Ty::Bool, span.clone());
                    HirExprKind::Unary {
                        op: HirUnaryOp::Not,
                        operand: Box::new(eq),
                    }
                } else {
                    call
                }
            } else {
                HirExprKind::Binary {
                    op: lower_binop(*op),
                    lhs: Box::new(l),
                    rhs: Box::new(r),
                }
            }
        }
        ExprKind::Range {
            start,
            end,
            inclusive,
        } => {
            let s = lower_expr(start, &Ty::Int, cx)?;
            let e = lower_expr(end, &Ty::Int, cx)?;
            HirExprKind::RangeNew {
                start: Box::new(s),
                end: Box::new(e),
                inclusive: *inclusive,
            }
        }
        ExprKind::Call { callee, args } => {
            // `EnumName.Variant(args)` lowers to an `EnumCreate`. The
            // callee is a `Field` whose receiver is the enum name.
            if let ExprKind::Field { receiver, name } = &callee.kind {
                if let Some(variant) = enum_variant_index(receiver, name, cx) {
                    let mut lowered = Vec::with_capacity(args.len());
                    for a in args {
                        lowered.push(lower_expr(a, &Ty::Error, cx)?);
                    }
                    return Ok(make_expr(
                        HirExprKind::EnumCreate {
                            variant,
                            args: lowered,
                        },
                        ty,
                        span,
                    ));
                }
            }
            // Compile-time reflection builtins. The callee is an unbound
            // identifier with a single type argument; the type checker
            // recorded the resolved type argument at the callee span. Carry
            // it into a dedicated node so MIR grounds it per monomorphization.
            if let ExprKind::Ident { name, .. } = &callee.kind {
                if cx.is_unbound_builtin(&callee.span) {
                    if name == "type_name" {
                        let arg_ty = cx.ty_at(&callee.span);
                        return Ok(make_expr(HirExprKind::TypeName(arg_ty), ty, span));
                    }
                    if name == "field_names" {
                        let arg_ty = cx.ty_at(&callee.span);
                        return Ok(make_expr(HirExprKind::FieldNames(arg_ty), ty, span));
                    }
                    if name == "field_types" {
                        let arg_ty = cx.ty_at(&callee.span);
                        return Ok(make_expr(HirExprKind::FieldTypes(arg_ty), ty, span));
                    }
                    if name == "variant_names" {
                        let arg_ty = cx.ty_at(&callee.span);
                        return Ok(make_expr(HirExprKind::VariantNames(arg_ty), ty, span));
                    }
                    if name == "variant_field_types" {
                        let arg_ty = cx.ty_at(&callee.span);
                        return Ok(make_expr(HirExprKind::VariantFieldTypes(arg_ty), ty, span));
                    }
                    if let Some(op) = ptr_builtin_op(name) {
                        let pointee = cx.ty_at(&callee.span);
                        let mut lowered = Vec::with_capacity(args.len());
                        for a in args {
                            lowered.push(lower_expr(a, &Ty::Error, cx)?);
                        }
                        return Ok(make_expr(
                            HirExprKind::PtrBuiltin {
                                op,
                                pointee,
                                args: lowered,
                            },
                            ty,
                            span,
                        ));
                    }
                    if let Some((op, has_type_arg)) = reflect_builtin_op(name) {
                        let type_arg = has_type_arg.then(|| cx.ty_at(&callee.span));
                        let mut lowered = Vec::with_capacity(args.len());
                        for a in args {
                            lowered.push(lower_expr(a, &Ty::Error, cx)?);
                        }
                        return Ok(make_expr(
                            HirExprKind::ReflectBuiltin {
                                op,
                                type_arg,
                                args: lowered,
                            },
                            ty,
                            span,
                        ));
                    }
                }
            }
            // Recognize the built in enum constructors `Some(x)`, `Ok(x)`,
            // and `Err(x)` so they lower to typed constructor nodes (and
            // then to `EnumCreate` in MIR) rather than ordinary calls.
            if let ExprKind::Ident { name, .. } = &callee.kind {
                if args.len() == 1 {
                    match name.as_str() {
                        "Some" => {
                            let inner = lower_expr(&args[0], &Ty::Error, cx)?;
                            return Ok(make_expr(HirExprKind::SomeCtor(Box::new(inner)), ty, span));
                        }
                        "Ok" => {
                            let inner = lower_expr(&args[0], &Ty::Error, cx)?;
                            return Ok(make_expr(HirExprKind::OkCtor(Box::new(inner)), ty, span));
                        }
                        "Err" => {
                            let inner = lower_expr(&args[0], &Ty::Error, cx)?;
                            return Ok(make_expr(HirExprKind::ErrCtor(Box::new(inner)), ty, span));
                        }
                        _ => {}
                    }
                }
            }
            // A callee that names a top-level function lowers to a
            // direct-call `Ident`. Lowering it through `lower_expr` would
            // turn it into a closure value (the value-position behavior),
            // which is wrong in callee position: the call site dispatches by
            // symbol, not through a closure object. A local, global, or
            // const callee keeps its normal value lowering (a closure value
            // dispatched indirectly).
            let c = match (&callee.kind, cx.fn_name_at(&callee.span)) {
                (ExprKind::Ident { .. }, Some(fn_name)) => make_expr(
                    HirExprKind::FnRef(fn_name),
                    cx.ty_at(&callee.span),
                    callee.span.clone(),
                ),
                _ => lower_expr(callee, &Ty::Error, cx)?,
            };
            let mut lowered = Vec::with_capacity(args.len());
            for a in args {
                lowered.push(lower_expr(a, &Ty::Error, cx)?);
            }
            // The built-in `print`/`println` accept any `ToString` value:
            // a non-`String` argument is routed through its `to_string`
            // method first, so `print(42)` and `print(point)` render
            // through the trait. The conversion is a `MethodCall`, which
            // MIR lowers to the value's per-type `to_string` symbol (and
            // monomorphization resolves a generic-parameter receiver to
            // the concrete impl). A `String` argument is left untouched so
            // the allocation-free literal fast path in codegen still runs.
            // Only the built-in (resolver-unbound) `print`/`println`
            // qualifies; an imported `std/io` function keeps its own
            // String-typed signature.
            if is_builtin_print(callee, cx) && lowered.len() == 1 {
                let arg = lowered.pop().expect("one argument checked above");
                let needs_conversion = !matches!(arg.ty.strip_self(), Ty::Str | Ty::Error);
                let arg = if needs_conversion {
                    let arg_span = arg.span.clone();
                    HirExpr {
                        kind: HirExprKind::MethodCall {
                            receiver: Box::new(arg),
                            name: "to_string".into(),
                            args: Vec::new(),
                        },
                        ty: Ty::Str,
                        span: arg_span,
                    }
                } else {
                    arg
                };
                lowered.push(arg);
            }
            HirExprKind::Call {
                callee: Box::new(c),
                args: lowered,
                type_args: cx.type_args_at(&callee.span),
            }
        }
        ExprKind::MethodCall {
            receiver,
            name,
            args,
            ..
        } => {
            // `EnumName.Variant(args)` constructs a payload variant. The
            // parser shapes it as a method call, so it is recognized here
            // before associated-call routing.
            if let Some(variant) = enum_variant_index(receiver, name, cx) {
                let mut lowered = Vec::with_capacity(args.len());
                for a in args {
                    lowered.push(lower_expr(a, &Ty::Error, cx)?);
                }
                return Ok(make_expr(
                    HirExprKind::EnumCreate {
                        variant,
                        args: lowered,
                    },
                    ty,
                    span,
                ));
            }
            // `module.func(args)` through a stdlib import alias lowers to an
            // ordinary call of the module's namespaced function, the same
            // symbol a selective import binds.
            if let Some(mangled) = module_qualified_fn(receiver, name, cx) {
                let mut lowered = Vec::with_capacity(args.len());
                for a in args {
                    lowered.push(lower_expr(a, &Ty::Error, cx)?);
                }
                let callee = make_expr(
                    HirExprKind::Ident(mangled),
                    Ty::Error,
                    receiver.span.clone(),
                );
                return Ok(make_expr(
                    HirExprKind::Call {
                        callee: Box::new(callee),
                        args: lowered,
                        type_args: Vec::new(),
                    },
                    ty,
                    span,
                ));
            }
            // `Type.func(args)` lowers to a receiverless associated call.
            // The type checker recorded the implementing type at the
            // receiver span; its presence as a type reference (not a value)
            // is what the type checker used to route this as an associated
            // function call.
            if is_type_ref_receiver(receiver, cx) {
                let self_ty = cx.ty_at(&receiver.span);
                let mut lowered = Vec::with_capacity(args.len());
                for a in args {
                    lowered.push(lower_expr(a, &Ty::Error, cx)?);
                }
                HirExprKind::AssocCall {
                    self_ty,
                    name: name.clone(),
                    args: lowered,
                }
            } else {
                let r = lower_expr(receiver, &Ty::Error, cx)?;
                let mut lowered = Vec::with_capacity(args.len());
                for a in args {
                    lowered.push(lower_expr(a, &Ty::Error, cx)?);
                }
                HirExprKind::MethodCall {
                    receiver: Box::new(r),
                    name: name.clone(),
                    args: lowered,
                }
            }
        }
        ExprKind::Field { receiver, name } => {
            // `EnumName.Variant` with no payload is a unit-variant
            // construction, not a field access.
            if let Some(variant) = enum_variant_index(receiver, name, cx) {
                HirExprKind::EnumCreate {
                    variant,
                    args: Vec::new(),
                }
            } else {
                let r = lower_expr(receiver, &Ty::Error, cx)?;
                HirExprKind::Field {
                    receiver: Box::new(r),
                    name: name.clone(),
                }
            }
        }
        ExprKind::Index { receiver, index } => {
            let r = lower_expr(receiver, &Ty::Error, cx)?;
            let i = lower_expr(index, &Ty::Int, cx)?;
            HirExprKind::Index {
                receiver: Box::new(r),
                index: Box::new(i),
            }
        }
        ExprKind::Try(inner) => return lower_try(inner, &ty, &span, cx),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let c = lower_expr(cond, &Ty::Bool, cx)?;
            let then_block = lower_block_to_block(then_branch, &ty, cx)?;
            let else_block = match else_branch.as_deref() {
                Some(ElseBranch::Block(b)) => Some(lower_block_to_block(b, &ty, cx)?),
                Some(ElseBranch::If(e)) => {
                    let lowered = lower_expr(e, &ty, cx)?;
                    Some(block_of_tail(lowered))
                }
                None => None,
            };
            HirExprKind::If {
                cond: Box::new(c),
                then_block,
                else_block,
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            let s = lower_expr(scrutinee, &Ty::Error, cx)?;
            let scrut_ty = s.ty.clone();
            let mut lowered = Vec::with_capacity(arms.len());
            for arm in arms {
                let mut pattern = lower_pattern(&arm.pattern, cx)?;
                reclassify_unit_variant(&mut pattern, &scrut_ty, cx);
                let guard = match &arm.guard {
                    Some(g) => Some(lower_expr(g, &Ty::Bool, cx)?),
                    None => None,
                };
                let body = lower_expr(&arm.body, &ty, cx)?;
                lowered.push(HirArm {
                    pattern,
                    guard,
                    body,
                    span: arm.span.clone(),
                });
            }
            HirExprKind::Match {
                scrutinee: Box::new(s),
                arms: lowered,
            }
        }
        ExprKind::Loop(b) => HirExprKind::Loop(lower_block_to_block(b, &Ty::Unit, cx)?),
        ExprKind::While { cond, body } => {
            let c = lower_expr(cond, &Ty::Bool, cx)?;
            let b = lower_block_to_block(body, &Ty::Unit, cx)?;
            HirExprKind::While {
                cond: Box::new(c),
                body: b,
            }
        }
        ExprKind::For {
            pattern: pat,
            iter,
            body,
        } => return lower_for(pat, iter, body, &ty, &span, cx),
        ExprKind::Lambda {
            params,
            ret: _,
            body,
            ..
        } => {
            let fn_ty = ty.clone();
            let (param_tys, ret_ty) = match &fn_ty {
                Ty::Function { params, ret } => (params.clone(), (**ret).clone()),
                _ => (Vec::new(), Ty::Error),
            };
            let mut lowered_params = Vec::with_capacity(params.len());
            for (i, p) in params.iter().enumerate() {
                let pty = param_tys.get(i).cloned().unwrap_or(Ty::Error);
                lowered_params.push((p.name.clone(), pty, p.span.clone()));
            }
            let body_block = match body {
                LambdaBody::Block(b) => lower_block_to_block(b, &ret_ty, cx)?,
                LambdaBody::Expr(e) => {
                    let lowered = lower_expr(e, &ret_ty, cx)?;
                    block_of_tail(lowered)
                }
            };
            HirExprKind::Lambda {
                params: lowered_params,
                ret: ret_ty,
                body: body_block,
            }
        }
        ExprKind::StructLit { name, fields, .. } => {
            let mut out = Vec::with_capacity(fields.len());
            for f in fields {
                let v = lower_expr(&f.value, &Ty::Error, cx)?;
                out.push((f.name.clone(), v));
            }
            HirExprKind::StructLit {
                name: name.clone(),
                fields: out,
            }
        }
    };
    let inner = HirExpr { kind, ty, span };
    // If the type checker recorded a `dyn Trait` coercion at this site,
    // wrap the concrete value in a `DynCoerce` node so MIR materializes
    // the fat pointer. The wrapper's type is the trait object type.
    if let Some(c) = cx.typed.types.lookup_coercion(&inner.span) {
        let dyn_ty = Ty::Dyn {
            name: c.trait_name.clone(),
            methods: c.methods.clone(),
        };
        let coerce_span = inner.span.clone();
        return Ok(HirExpr {
            kind: HirExprKind::DynCoerce {
                trait_name: c.trait_name.clone(),
                methods: c.methods.clone(),
                concrete_ty: c.concrete_ty.clone(),
                value: Box::new(inner),
            },
            ty: dyn_ty,
            span: coerce_span,
        });
    }
    Ok(inner)
}

/// Lower a set literal `{e1, e2, ...}` to a block that builds the set
/// with the `Set.new()` associated constructor and one `add` call per
/// element:
///
/// ```text
/// { let __set = Set.new(); __set.add(e1); __set.add(e2); ...; __set }
/// ```
///
/// The set type comes from the type checker (recorded at the literal's
/// span), so `Set.new()` is an `AssocCall` on that concrete `Set<T>` and
/// each `add` is an ordinary method call. Monomorphization resolves both
/// to the element type's impl symbols, exactly as for hand-written code.
fn lower_set_lit(
    items: &[Expr],
    set_ty: &Ty,
    span: &Span,
    cx: &LowerCtx<'_>,
) -> Result<HirExpr, RavenError> {
    let elem_ty = match set_ty.strip_self() {
        Ty::Struct { args, .. } => args.first().cloned().unwrap_or(Ty::Error),
        _ => Ty::Error,
    };
    let name = cx.fresh("set");
    let ctor = make_expr(
        HirExprKind::AssocCall {
            self_ty: set_ty.clone(),
            name: "new".into(),
            args: Vec::new(),
        },
        set_ty.clone(),
        span.clone(),
    );
    let mut stmts = vec![let_stmt(&name, set_ty.clone(), ctor, span.clone())];
    for it in items {
        let value = lower_expr(it, &elem_ty, cx)?;
        let recv = ident_expr(&name, set_ty.clone(), it.span.clone());
        let add = make_expr(
            HirExprKind::MethodCall {
                receiver: Box::new(recv),
                name: "add".into(),
                args: vec![value],
            },
            Ty::Unit,
            it.span.clone(),
        );
        stmts.push(HirStmt {
            kind: HirStmtKind::Expr(add),
            span: it.span.clone(),
        });
    }
    let tail = ident_expr(&name, set_ty.clone(), span.clone());
    let block = HirBlock {
        stmts,
        tail: Some(Box::new(tail)),
        ty: set_ty.clone(),
        span: span.clone(),
    };
    Ok(make_expr(
        HirExprKind::Block(block),
        set_ty.clone(),
        span.clone(),
    ))
}

/// Lower a map literal `[k1: v1, ...]` (and the empty `[:]`) to a block
/// that builds the map with the `Map.new()` associated constructor and
/// one `set` call per pair:
///
/// ```text
/// { let __map = Map.new(); __map.set(k1, v1); ...; __map }
/// ```
fn lower_map_lit(
    pairs: &[(Expr, Expr)],
    map_ty: &Ty,
    span: &Span,
    cx: &LowerCtx<'_>,
) -> Result<HirExpr, RavenError> {
    let (key_ty, val_ty) = match map_ty.strip_self() {
        Ty::Struct { args, .. } => (
            args.first().cloned().unwrap_or(Ty::Error),
            args.get(1).cloned().unwrap_or(Ty::Error),
        ),
        _ => (Ty::Error, Ty::Error),
    };
    let name = cx.fresh("map");
    let ctor = make_expr(
        HirExprKind::AssocCall {
            self_ty: map_ty.clone(),
            name: "new".into(),
            args: Vec::new(),
        },
        map_ty.clone(),
        span.clone(),
    );
    let mut stmts = vec![let_stmt(&name, map_ty.clone(), ctor, span.clone())];
    for (k, v) in pairs {
        let pair_span = Span::new(
            k.span.file.clone(),
            k.span.start.min(v.span.start),
            k.span.end.max(v.span.end),
            k.span.line,
            k.span.col,
        );
        let key = lower_expr(k, &key_ty, cx)?;
        let value = lower_expr(v, &val_ty, cx)?;
        let recv = ident_expr(&name, map_ty.clone(), pair_span.clone());
        let set = make_expr(
            HirExprKind::MethodCall {
                receiver: Box::new(recv),
                name: "set".into(),
                args: vec![key, value],
            },
            Ty::Unit,
            pair_span.clone(),
        );
        stmts.push(HirStmt {
            kind: HirStmtKind::Expr(set),
            span: pair_span,
        });
    }
    let tail = ident_expr(&name, map_ty.clone(), span.clone());
    let block = HirBlock {
        stmts,
        tail: Some(Box::new(tail)),
        ty: map_ty.clone(),
        span: span.clone(),
    };
    Ok(make_expr(
        HirExprKind::Block(block),
        map_ty.clone(),
        span.clone(),
    ))
}

/// Map a raw-pointer FFI builtin name to its op, or `None` for any other
/// identifier.
fn ptr_builtin_op(name: &str) -> Option<PtrBuiltinOp> {
    Some(match name {
        "__ptr_alloc" => PtrBuiltinOp::Alloc,
        "__ptr_free" => PtrBuiltinOp::Free,
        "__ptr_load" => PtrBuiltinOp::Load,
        "__ptr_store" => PtrBuiltinOp::Store,
        "__ptr_offset" => PtrBuiltinOp::Offset,
        "__ptr_is_null" => PtrBuiltinOp::IsNull,
        "__ptr_null" => PtrBuiltinOp::Null,
        _ => return None,
    })
}

/// Map a runtime reflection builtin name to its op, or `None` for any
/// other identifier. The second element is true when the builtin carries a
/// type argument recorded at the callee span (`to_any<T>`, `cast<T>`).
fn reflect_builtin_op(name: &str) -> Option<(ReflectBuiltinOp, bool)> {
    Some(match name {
        "to_any" => (ReflectBuiltinOp::ToAny, true),
        "cast" => (ReflectBuiltinOp::Cast, true),
        "type_name_of" => (ReflectBuiltinOp::TypeNameOf, false),
        "field_names_of" => (ReflectBuiltinOp::FieldNamesOf, false),
        "get_field" => (ReflectBuiltinOp::GetField, false),
        "set_field" => (ReflectBuiltinOp::SetField, false),
        _ => return None,
    })
}

fn lower_unop(op: UnaryOp) -> HirUnaryOp {
    match op {
        UnaryOp::Neg => HirUnaryOp::Neg,
        UnaryOp::Not => HirUnaryOp::Not,
        UnaryOp::Ref => HirUnaryOp::Ref,
    }
}

/// The head identity used to match a type against an `Eq` impl's self type,
/// or `None` for a type whose `==` uses a native compare instead of `equals`
/// (the primitives, and `String`, which has its own byte-equality path).
/// `Set`/`Map` are ordinary structs, so they match through the `Struct` arm.
fn eq_head(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Struct { name, .. } | Ty::Enum { name, .. } => Some(name.clone()),
        Ty::Option(_) => Some("Option".to_string()),
        Ty::Result(_, _) => Some("Result".to_string()),
        Ty::List(_) => Some("List".to_string()),
        _ => None,
    }
}

/// Whether `ty` is a type that implements `Eq` (an impl on it provides an
/// `equals` method), so `==`/`!=` on it should dispatch to that method rather
/// than compare the heap pointers. Matched by the type's head, which covers
/// the concrete and generic (`impl<T: Eq> Eq for Pair<T>` / `Option<T>`)
/// cases the derive pass and hand-written impls produce.
fn type_has_equals(env: &crate::tycheck::TypeEnv, ty: &Ty) -> bool {
    let Some(head) = eq_head(ty) else {
        return false;
    };
    env.impls.iter().any(|imp| {
        imp.methods.contains_key("equals")
            && eq_head(&imp.self_ty).as_deref() == Some(head.as_str())
    })
}

fn lower_binop(op: BinaryOp) -> HirBinaryOp {
    match op {
        BinaryOp::Add => HirBinaryOp::Add,
        BinaryOp::Sub => HirBinaryOp::Sub,
        BinaryOp::Mul => HirBinaryOp::Mul,
        BinaryOp::Div => HirBinaryOp::Div,
        BinaryOp::Mod => HirBinaryOp::Mod,
        BinaryOp::Eq => HirBinaryOp::Eq,
        BinaryOp::Ne => HirBinaryOp::Ne,
        BinaryOp::Lt => HirBinaryOp::Lt,
        BinaryOp::Le => HirBinaryOp::Le,
        BinaryOp::Gt => HirBinaryOp::Gt,
        BinaryOp::Ge => HirBinaryOp::Ge,
        BinaryOp::And => HirBinaryOp::And,
        BinaryOp::Or => HirBinaryOp::Or,
        BinaryOp::BitAnd => HirBinaryOp::BitAnd,
        BinaryOp::BitOr => HirBinaryOp::BitOr,
        BinaryOp::BitXor => HirBinaryOp::BitXor,
        BinaryOp::Shl => HirBinaryOp::Shl,
        BinaryOp::Shr => HirBinaryOp::Shr,
    }
}

/// Lower a `?` expression. The result type comes from the type checker
/// and tells us whether we are propagating a `Result` or an `Option`.
fn lower_try(
    inner: &Expr,
    result_ty: &Ty,
    span: &Span,
    cx: &LowerCtx<'_>,
) -> Result<HirExpr, RavenError> {
    let inner_lowered = lower_expr(inner, &Ty::Error, cx)?;
    let inner_ty = inner_lowered.ty.clone();
    let temp_v = cx.fresh("try_v");
    let temp_e = cx.fresh("try_e");

    let (ok_arm, err_arm) = match &inner_ty {
        Ty::Result(t, e) => {
            let ok_body = ident_expr(&temp_v, (**t).clone(), span.clone());
            let err_payload = ident_expr(&temp_e, (**e).clone(), span.clone());
            let err_ctor = make_expr(
                HirExprKind::ErrCtor(Box::new(err_payload)),
                inner_ty.clone(),
                span.clone(),
            );
            let return_expr = make_expr(
                HirExprKind::Return(Some(Box::new(err_ctor))),
                Ty::Error,
                span.clone(),
            );
            let ok_arm = HirArm {
                pattern: ctor_pat("Ok", vec![bind_pat(&temp_v, span.clone())], span.clone()),
                guard: None,
                body: ok_body,
                span: span.clone(),
            };
            let err_arm = HirArm {
                pattern: ctor_pat("Err", vec![bind_pat(&temp_e, span.clone())], span.clone()),
                guard: None,
                body: return_expr,
                span: span.clone(),
            };
            (ok_arm, err_arm)
        }
        Ty::Option(t) => {
            let some_body = ident_expr(&temp_v, (**t).clone(), span.clone());
            let none_ctor = make_expr(HirExprKind::NoneCtor, inner_ty.clone(), span.clone());
            let return_expr = make_expr(
                HirExprKind::Return(Some(Box::new(none_ctor))),
                Ty::Error,
                span.clone(),
            );
            let ok_arm = HirArm {
                pattern: ctor_pat("Some", vec![bind_pat(&temp_v, span.clone())], span.clone()),
                guard: None,
                body: some_body,
                span: span.clone(),
            };
            let err_arm = HirArm {
                pattern: ctor_pat("None", Vec::new(), span.clone()),
                guard: None,
                body: return_expr,
                span: span.clone(),
            };
            (ok_arm, err_arm)
        }
        _ => {
            return Err(super::ty_error(
                "`?` operator requires Result or Option receiver",
                span,
            ));
        }
    };

    let match_expr = make_expr(
        HirExprKind::Match {
            scrutinee: Box::new(inner_lowered),
            arms: vec![ok_arm, err_arm],
        },
        result_ty.clone(),
        span.clone(),
    );
    Ok(match_expr)
}

fn bind_pat(name: &str, span: Span) -> HirPattern {
    HirPattern {
        kind: HirPatternKind::Binding(name.to_string()),
        span,
    }
}

fn ctor_pat(name: &str, elements: Vec<HirPattern>, span: Span) -> HirPattern {
    HirPattern {
        kind: HirPatternKind::Constructor {
            name: Some(name.to_string()),
            elements,
        },
        span,
    }
}

/// Lower a `for pat in iter { body }` into a concrete counter loop.
///
/// Two iterable forms are supported here without any iterator object:
///
/// * A range `start..end` (exclusive) or `start..=end` (inclusive) is
///   lowered to a counter loop over the half-open or closed integer
///   interval. The endpoints are each evaluated once into a local.
/// * A `List<T>` value is lowered to an index loop driven by the list's
///   `len()` and element indexing (issue #138). The list expression is
///   evaluated once into a local.
///
/// Both forms produce the same shape:
///
/// ```text
/// {
///     let __start = <start>;          // range: start; list: 0
///     let __end = <end>;              // range: end;   list: __list.len()
///     let __i = __start;
///     let __first = true;
///     loop {
///         // The increment sits at the top of the loop body so that a
///         // user `continue` (which jumps to the loop header) still
///         // advances the counter before the next iteration. The
///         // `__first` flag skips the increment on the very first pass so
///         // the counter starts at `__start`.
///         if __first { __first = false } else { __i = __i + 1 }
///         if __i >= __end { break }   // `>` for an inclusive range
///         let pat = __i;              // range: __i; list: __list[__i]
///         <body>
///     }
/// }
/// ```
///
/// `break` exits to the loop continuation as usual, and `continue`
/// re-enters the loop header, which is the increment-and-test step, so
/// it never skips the counter advance. Iteration over any type other
/// than a range or a `List<T>` is rejected by the type checker before
/// lowering, so the arbitrary-iterator path (the `Iterator` trait, issue
/// #119) does not reach here.
fn lower_for(
    pat: &crate::ast::Pattern,
    iter: &Expr,
    body: &AstBlock,
    _result_ty: &Ty,
    span: &Span,
    cx: &LowerCtx<'_>,
) -> Result<HirExpr, RavenError> {
    // A `start..end` / `start..=end` source range lowers to a counter
    // loop directly from its endpoints; any other iterable is treated as
    // a list value driven by `len()` and indexing.
    if let ExprKind::Range {
        start,
        end,
        inclusive,
    } = &iter.kind
    {
        let start_lowered = lower_expr(start, &Ty::Int, cx)?;
        let end_lowered = lower_expr(end, &Ty::Int, cx)?;
        return Ok(lower_counter_for(
            pat,
            start_lowered,
            end_lowered,
            *inclusive,
            None,
            Ty::Int,
            body,
            span,
            cx,
        ));
    }

    let source_expr = lower_expr(iter, &Ty::Error, cx)?;
    // A non-range, non-list iterable is any value whose type implements
    // `Iterator<T>`. Drive it through repeated `next()` calls.
    if !matches!(source_expr.ty.strip_self(), Ty::List(_)) {
        return Ok(lower_iterator_for(pat, source_expr, body, span, cx));
    }
    let list_expr = source_expr;
    let element_ty = match list_expr.ty.strip_self() {
        Ty::List(t) => (**t).clone(),
        _ => Ty::Error,
    };
    let list_ty = list_expr.ty.clone();
    let list_name = cx.fresh("list");
    let list_let = let_stmt(&list_name, list_ty.clone(), list_expr, span.clone());

    // start = 0, end = __list.len(), element = __list[__i].
    let start_lowered = make_expr(HirExprKind::Int(0), Ty::Int, span.clone());
    let len_recv = ident_expr(&list_name, list_ty, span.clone());
    let end_lowered = make_expr(
        HirExprKind::MethodCall {
            receiver: Box::new(len_recv),
            name: "len".into(),
            args: Vec::new(),
        },
        Ty::Int,
        span.clone(),
    );
    let counter_loop = lower_counter_for(
        pat,
        start_lowered,
        end_lowered,
        false,
        Some(&list_name),
        element_ty,
        body,
        span,
        cx,
    );
    let block = HirBlock {
        stmts: vec![list_let],
        tail: Some(Box::new(counter_loop)),
        ty: Ty::Unit,
        span: span.clone(),
    };
    Ok(make_expr(HirExprKind::Block(block), Ty::Unit, span.clone()))
}

/// Lower a `for` loop whose source is an arbitrary iterator (any value
/// whose type implements `Iterator<T>`). The loop drives the iterator by
/// calling `next()` until it yields `None`:
///
/// ```text
/// {
///     let __it = <source>;
///     loop {
///         match __it.next() {
///             Some(<binding>) => { <body> },
///             None => break,
///         }
///     }
/// }
/// ```
///
/// The `next` call is an ordinary method call, so monomorphization
/// resolves it to the concrete adapter's `next` symbol and the closures
/// captured by `map`/`filter`/... are dispatched statically. There is no
/// boxing and no per-stage allocation: a chained pipeline runs in a
/// single pass driven by this loop.
fn lower_iterator_for(
    pat: &crate::ast::Pattern,
    source: HirExpr,
    body: &AstBlock,
    span: &Span,
    cx: &LowerCtx<'_>,
) -> HirExpr {
    // The element type recorded by the type checker at the pattern span.
    let element_ty = cx
        .typed
        .types
        .lookup(&pat.span)
        .cloned()
        .unwrap_or(Ty::Error);

    let it_name = cx.fresh("it");
    let it_ty = source.ty.clone();
    let it_let = let_stmt(&it_name, it_ty.clone(), source, span.clone());

    // __it.next() : Option<element_ty>
    let next_call = make_expr(
        HirExprKind::MethodCall {
            receiver: Box::new(ident_expr(&it_name, it_ty, span.clone())),
            name: "next".into(),
            args: Vec::new(),
        },
        Ty::Option(Box::new(element_ty.clone())),
        span.clone(),
    );

    // Some(<binding>) => { <body> }
    let bind_name = pattern_binding_name(pat).unwrap_or_else(|| cx.fresh("loopvar"));
    let some_pat = HirPattern {
        kind: HirPatternKind::Constructor {
            name: Some("Some".into()),
            elements: vec![HirPattern {
                kind: HirPatternKind::Binding(bind_name),
                span: span.clone(),
            }],
        },
        span: span.clone(),
    };
    let body_block = lower_block_to_block(body, &Ty::Unit, cx).unwrap_or_else(|_| HirBlock {
        stmts: Vec::new(),
        tail: None,
        ty: Ty::Unit,
        span: span.clone(),
    });
    let some_arm = HirArm {
        pattern: some_pat,
        guard: None,
        body: make_expr(HirExprKind::Block(body_block), Ty::Unit, span.clone()),
        span: span.clone(),
    };

    // None => break
    let none_pat = HirPattern {
        kind: HirPatternKind::Constructor {
            name: Some("None".into()),
            elements: Vec::new(),
        },
        span: span.clone(),
    };
    let none_arm = HirArm {
        pattern: none_pat,
        guard: None,
        body: make_expr(HirExprKind::Break(None), Ty::Error, span.clone()),
        span: span.clone(),
    };

    let match_expr = make_expr(
        HirExprKind::Match {
            scrutinee: Box::new(next_call),
            arms: vec![some_arm, none_arm],
        },
        Ty::Unit,
        span.clone(),
    );
    let loop_block = HirBlock {
        stmts: vec![HirStmt {
            kind: HirStmtKind::Expr(match_expr),
            span: span.clone(),
        }],
        tail: None,
        ty: Ty::Unit,
        span: span.clone(),
    };
    let loop_expr = make_expr(HirExprKind::Loop(loop_block), Ty::Unit, span.clone());

    let block = HirBlock {
        stmts: vec![it_let],
        tail: Some(Box::new(loop_expr)),
        ty: Ty::Unit,
        span: span.clone(),
    };
    make_expr(HirExprKind::Block(block), Ty::Unit, span.clone())
}

/// Build the counter-loop body shared by the range and list for-loop
/// forms. `start`/`end` are the already-lowered bounds, `inclusive`
/// selects `>` over `>=` for the break test, and `list_name` (when
/// `Some`) makes the per-iteration binding `__list[__i]` instead of the
/// raw counter `__i`. `element_ty` is the loop variable's type.
#[allow(clippy::too_many_arguments)]
fn lower_counter_for(
    pat: &crate::ast::Pattern,
    start: HirExpr,
    end: HirExpr,
    inclusive: bool,
    list_name: Option<&str>,
    element_ty: Ty,
    body: &AstBlock,
    span: &Span,
    cx: &LowerCtx<'_>,
) -> HirExpr {
    let i_name = cx.fresh("i");
    let end_name = cx.fresh("end");
    let first_name = cx.fresh("first");

    let end_let = let_stmt(&end_name, Ty::Int, end, span.clone());
    let i_let = let_stmt(&i_name, Ty::Int, start, span.clone());
    let first_let = let_stmt(
        &first_name,
        Ty::Bool,
        make_expr(HirExprKind::Bool(true), Ty::Bool, span.clone()),
        span.clone(),
    );

    // if __first { __first = false } else { __i = __i + 1 }
    let set_first_false = assign_stmt(
        HirAssignTarget::Ident {
            name: first_name.clone(),
            span: span.clone(),
        },
        make_expr(HirExprKind::Bool(false), Ty::Bool, span.clone()),
        span.clone(),
    );
    let inc = assign_stmt(
        HirAssignTarget::Ident {
            name: i_name.clone(),
            span: span.clone(),
        },
        make_expr(
            HirExprKind::Binary {
                op: HirBinaryOp::Add,
                lhs: Box::new(ident_expr(&i_name, Ty::Int, span.clone())),
                rhs: Box::new(make_expr(HirExprKind::Int(1), Ty::Int, span.clone())),
            },
            Ty::Int,
            span.clone(),
        ),
        span.clone(),
    );
    // For an inclusive range the counter must not be incremented past `end`:
    // a loop to `i64::MAX` would wrap to `i64::MIN` and run forever. Once `__i`
    // has reached `end`, break instead of incrementing. This sits in the
    // advance step at the top of the loop, so it also holds when the body uses
    // `continue`. An exclusive range stops at `end` before ever incrementing
    // into it, so it keeps the plain increment.
    let step_block = if inclusive {
        let at_end = make_expr(
            HirExprKind::Binary {
                op: HirBinaryOp::Eq,
                lhs: Box::new(ident_expr(&i_name, Ty::Int, span.clone())),
                rhs: Box::new(ident_expr(&end_name, Ty::Int, span.clone())),
            },
            Ty::Bool,
            span.clone(),
        );
        let break_at_end = HirStmt {
            kind: HirStmtKind::Expr(make_expr(HirExprKind::Break(None), Ty::Error, span.clone())),
            span: span.clone(),
        };
        let guarded = make_expr(
            HirExprKind::If {
                cond: Box::new(at_end),
                then_block: HirBlock {
                    stmts: vec![break_at_end],
                    tail: None,
                    ty: Ty::Unit,
                    span: span.clone(),
                },
                else_block: Some(block_of_stmt(inc, span)),
            },
            Ty::Unit,
            span.clone(),
        );
        block_of_stmt(
            HirStmt {
                kind: HirStmtKind::Expr(guarded),
                span: span.clone(),
            },
            span,
        )
    } else {
        block_of_stmt(inc, span)
    };
    let advance = make_expr(
        HirExprKind::If {
            cond: Box::new(ident_expr(&first_name, Ty::Bool, span.clone())),
            then_block: block_of_stmt(set_first_false, span),
            else_block: Some(step_block),
        },
        Ty::Unit,
        span.clone(),
    );

    // if __i >= __end { break }   (or `>` for an inclusive range)
    let break_op = if inclusive {
        HirBinaryOp::Gt
    } else {
        HirBinaryOp::Ge
    };
    let break_cond = make_expr(
        HirExprKind::Binary {
            op: break_op,
            lhs: Box::new(ident_expr(&i_name, Ty::Int, span.clone())),
            rhs: Box::new(ident_expr(&end_name, Ty::Int, span.clone())),
        },
        Ty::Bool,
        span.clone(),
    );
    let break_stmt = HirStmt {
        kind: HirStmtKind::Expr(make_expr(HirExprKind::Break(None), Ty::Error, span.clone())),
        span: span.clone(),
    };
    let break_guard = make_expr(
        HirExprKind::If {
            cond: Box::new(break_cond),
            then_block: HirBlock {
                stmts: vec![break_stmt],
                tail: None,
                ty: Ty::Unit,
                span: span.clone(),
            },
            else_block: None,
        },
        Ty::Unit,
        span.clone(),
    );

    // The per-iteration element: `__list[__i]` for a list, else `__i`.
    let element_init = match list_name {
        Some(name) => make_expr(
            HirExprKind::Index {
                receiver: Box::new(ident_expr(
                    name,
                    Ty::List(Box::new(element_ty.clone())),
                    span.clone(),
                )),
                index: Box::new(ident_expr(&i_name, Ty::Int, span.clone())),
            },
            element_ty.clone(),
            span.clone(),
        ),
        None => ident_expr(&i_name, element_ty.clone(), span.clone()),
    };

    // The user body becomes the trailing statements of the loop body.
    let body_block = lower_block_to_block(body, &Ty::Unit, cx).unwrap_or_else(|_| HirBlock {
        stmts: Vec::new(),
        tail: None,
        ty: Ty::Unit,
        span: span.clone(),
    });
    let body_expr = make_expr(HirExprKind::Block(body_block), Ty::Unit, span.clone());

    // Bind the loop element to the pattern. `for x in ...` binds a single name
    // directly. A destructuring pattern (`for Point { x, y } in ...`) cannot
    // lower to a plain `let`, since Raven has no pattern `let`: bind a
    // temporary and `match` it so the pattern's variables come into scope for
    // the body. A refutable pattern gets a wildcard arm that skips an element
    // that does not match.
    // Dispatch on the lowered pattern, not the surface syntax: an identifier
    // like `None` is a nullary constructor, not a binding, and must take the
    // match path so `for None in xs` filters instead of rebinding.
    let simple_let = |name: &str, ty: Ty, init: HirExpr, body: HirExpr| {
        vec![
            let_stmt(name, ty, init, span.clone()),
            HirStmt {
                kind: HirStmtKind::Expr(body),
                span: span.clone(),
            },
        ]
    };
    let bind_and_body: Vec<HirStmt> = match lower_pattern(pat, cx) {
        Ok(hir_pat) => {
            if let HirPatternKind::Binding(name) = &hir_pat.kind {
                let name = name.clone();
                simple_let(&name, element_ty, element_init, body_expr)
            } else if matches!(hir_pat.kind, HirPatternKind::Wildcard) {
                simple_let(&cx.fresh("loopvar"), element_ty, element_init, body_expr)
            } else {
                let refutable = hir_pattern_is_refutable(&hir_pat);
                let loopvar = cx.fresh("loopvar");
                let bind_let = let_stmt(&loopvar, element_ty.clone(), element_init, span.clone());
                let scrutinee = ident_expr(&loopvar, element_ty, span.clone());
                let mut arms = vec![HirArm {
                    pattern: hir_pat,
                    guard: None,
                    body: body_expr,
                    span: span.clone(),
                }];
                if refutable {
                    arms.push(HirArm {
                        pattern: HirPattern {
                            kind: HirPatternKind::Wildcard,
                            span: span.clone(),
                        },
                        guard: None,
                        body: make_expr(
                            HirExprKind::Block(HirBlock {
                                stmts: Vec::new(),
                                tail: None,
                                ty: Ty::Unit,
                                span: span.clone(),
                            }),
                            Ty::Unit,
                            span.clone(),
                        ),
                        span: span.clone(),
                    });
                }
                let match_expr = make_expr(
                    HirExprKind::Match {
                        scrutinee: Box::new(scrutinee),
                        arms,
                    },
                    Ty::Unit,
                    span.clone(),
                );
                vec![
                    bind_let,
                    HirStmt {
                        kind: HirStmtKind::Expr(match_expr),
                        span: span.clone(),
                    },
                ]
            }
        }
        // A pattern that did not lower: evaluate the element into a throwaway
        // and run the body, so the loop still iterates.
        Err(_) => simple_let(&cx.fresh("loopvar"), element_ty, element_init, body_expr),
    };

    let mut loop_stmts = vec![
        HirStmt {
            kind: HirStmtKind::Expr(advance),
            span: span.clone(),
        },
        HirStmt {
            kind: HirStmtKind::Expr(break_guard),
            span: span.clone(),
        },
    ];
    loop_stmts.extend(bind_and_body);
    let loop_block = HirBlock {
        stmts: loop_stmts,
        tail: None,
        ty: Ty::Unit,
        span: span.clone(),
    };
    let loop_expr = make_expr(HirExprKind::Loop(loop_block), Ty::Unit, span.clone());

    let block = HirBlock {
        // `i_let` (the start bound) before `end_let` so a `start..end` range
        // evaluates its bounds left to right, matching source order.
        stmts: vec![i_let, end_let, first_let],
        tail: Some(Box::new(loop_expr)),
        ty: Ty::Unit,
        span: span.clone(),
    };
    make_expr(HirExprKind::Block(block), Ty::Unit, span.clone())
}

/// Wrap a single statement in a block whose value is `Unit`. Used for the
/// `if`/`else` arms of the for-loop counter advance.
fn block_of_stmt(stmt: HirStmt, span: &Span) -> HirBlock {
    HirBlock {
        stmts: vec![stmt],
        tail: None,
        ty: Ty::Unit,
        span: span.clone(),
    }
}

/// Extract the single binding name from a `for` loop pattern, when the
/// pattern is a plain binding (`for x in ...`). Returns `None` for
/// wildcard or richer patterns so the caller can synthesize a fresh
/// loop-variable name instead.
fn pattern_binding_name(pat: &crate::ast::Pattern) -> Option<String> {
    use crate::ast::PatternKind;
    match &pat.kind {
        PatternKind::Ident(name) => Some(name.clone()),
        _ => None,
    }
}

/// Whether a pattern can fail to match a value of its scrutinee type, so a
/// `for` loop desugared to a `match` needs a wildcard arm to skip an element
/// that does not match. A binding or wildcard always matches; an enum
/// constructor, a literal, or a range can fail; a struct is refutable only when
/// one of its field patterns is. The check recurses so a nested refutable
/// pattern is not missed.
fn hir_pattern_is_refutable(pat: &HirPattern) -> bool {
    match &pat.kind {
        HirPatternKind::Wildcard | HirPatternKind::Binding(_) => false,
        HirPatternKind::Literal(_) | HirPatternKind::Range { .. } => true,
        HirPatternKind::Constructor { .. } => true,
        HirPatternKind::Struct { fields, .. } => fields
            .iter()
            .any(|f| f.pattern.as_ref().is_some_and(hir_pattern_is_refutable)),
    }
}

/// Lower a compound assignment `target op= value` into a flat
/// assignment of `target = target op value`. For non-identifier targets
/// we first evaluate the LHS components into fresh locals so they are
/// not evaluated twice.
pub(crate) fn lower_compound_assign(
    target: &Expr,
    op: HirBinaryOp,
    value: &Expr,
    span: &Span,
    cx: &LowerCtx<'_>,
) -> Result<Vec<HirStmt>, RavenError> {
    let target_ty = cx.ty_at(&target.span);
    let value_lowered = lower_expr(value, &target_ty, cx)?;
    match &target.kind {
        // A plain identifier, or `self` as a bare target. Both bind by a
        // name string (`self` uses the fixed `self` key). `x op= v`
        // becomes `x = x op v` with the load reading the same name.
        ExprKind::Ident { .. } | ExprKind::SelfLower => {
            let name = match &target.kind {
                ExprKind::Ident { name, .. } => name.clone(),
                _ => "self".to_string(),
            };
            // A module-level `let` reads and writes through its global symbol,
            // not a local slot. `g += v` must load with `GlobalGet` and store
            // to a `Global` target the same way a plain read and write do;
            // otherwise the load resolved to a missing local and came back as
            // a Unit value.
            let (load_kind, store) = match module_global_name(&target.span, cx) {
                Some(global) => (
                    HirExprKind::GlobalGet(global.clone()),
                    HirAssignTarget::Global { name: global },
                ),
                None => (
                    target_kind_for_name(&target.kind, &name),
                    HirAssignTarget::Ident {
                        name,
                        span: target.span.clone(),
                    },
                ),
            };
            let load = make_expr(load_kind, target_ty.clone(), target.span.clone());
            let combined = make_expr(
                HirExprKind::Binary {
                    op,
                    lhs: Box::new(load),
                    rhs: Box::new(value_lowered),
                },
                target_ty,
                span.clone(),
            );
            Ok(vec![assign_stmt(store, combined, span.clone())])
        }
        ExprKind::Field { receiver, name } => {
            // obj.field op= v  ->  let __recv = obj; __recv.field = __recv.field op v
            let recv_ty = cx.ty_at(&receiver.span);
            let recv_lowered = lower_expr(receiver, &Ty::Error, cx)?;
            let recv_name = cx.fresh("recv");
            let let_recv = let_stmt(
                &recv_name,
                recv_ty.clone(),
                recv_lowered,
                receiver.span.clone(),
            );
            let recv_ref = ident_expr(&recv_name, recv_ty.clone(), receiver.span.clone());
            let recv_ref_for_load = ident_expr(&recv_name, recv_ty, receiver.span.clone());
            let load = make_expr(
                HirExprKind::Field {
                    receiver: Box::new(recv_ref_for_load),
                    name: name.clone(),
                },
                target_ty.clone(),
                target.span.clone(),
            );
            let combined = make_expr(
                HirExprKind::Binary {
                    op,
                    lhs: Box::new(load),
                    rhs: Box::new(value_lowered),
                },
                target_ty,
                span.clone(),
            );
            let assign = assign_stmt(
                HirAssignTarget::Field {
                    recv: recv_ref,
                    name: name.clone(),
                },
                combined,
                span.clone(),
            );
            Ok(vec![let_recv, assign])
        }
        ExprKind::Index { receiver, index } => {
            // arr[i] op= v  ->  let __arr = arr; let __idx = i; __arr[__idx] = __arr[__idx] op v
            let recv_ty = cx.ty_at(&receiver.span);
            let idx_ty = cx.ty_at(&index.span);
            let recv_lowered = lower_expr(receiver, &Ty::Error, cx)?;
            let idx_lowered = lower_expr(index, &Ty::Int, cx)?;
            let recv_name = cx.fresh("recv");
            let idx_name = cx.fresh("idx");
            let let_recv = let_stmt(
                &recv_name,
                recv_ty.clone(),
                recv_lowered,
                receiver.span.clone(),
            );
            let let_idx = let_stmt(&idx_name, idx_ty.clone(), idx_lowered, index.span.clone());
            let recv_ref = ident_expr(&recv_name, recv_ty.clone(), receiver.span.clone());
            let idx_ref = ident_expr(&idx_name, idx_ty.clone(), index.span.clone());
            let recv_ref_load = ident_expr(&recv_name, recv_ty.clone(), receiver.span.clone());
            let idx_ref_load = ident_expr(&idx_name, idx_ty, index.span.clone());
            let load = make_expr(
                HirExprKind::Index {
                    receiver: Box::new(recv_ref_load),
                    index: Box::new(idx_ref_load),
                },
                target_ty.clone(),
                target.span.clone(),
            );
            let combined = make_expr(
                HirExprKind::Binary {
                    op,
                    lhs: Box::new(load),
                    rhs: Box::new(value_lowered),
                },
                target_ty,
                span.clone(),
            );
            let assign = assign_stmt(
                HirAssignTarget::Index {
                    recv: recv_ref,
                    index: idx_ref,
                },
                combined,
                span.clone(),
            );
            Ok(vec![let_recv, let_idx, assign])
        }
        _ => Err(super::ty_error(
            "invalid left-hand side in compound assignment",
            &target.span,
        )),
    }
}

/// Build the HIR load kind for a name-rooted compound-assignment target.
/// A `self` receiver loads through `SelfValue` so MIR resolves the fixed
/// `self` local; any other name loads as a plain identifier reference.
fn target_kind_for_name(kind: &ExprKind, name: &str) -> HirExprKind {
    match kind {
        ExprKind::SelfLower => HirExprKind::SelfValue,
        _ => HirExprKind::Ident(name.to_string()),
    }
}

/// Convenience used by `lower_stmt::lower_assign_plain` to build a HIR
/// statement.
pub(crate) fn build_plain_assign(target: HirAssignTarget, value: HirExpr, span: Span) -> HirStmt {
    assign_stmt(target, value, span)
}

/// Lower a target expression for a plain `=` assignment to a
/// `HirAssignTarget`.
pub(crate) fn lower_assign_target(
    expr: &Expr,
    cx: &LowerCtx<'_>,
) -> Result<HirAssignTarget, RavenError> {
    match &expr.kind {
        ExprKind::Ident { name, .. } => {
            // Assigning to a mutable module-level `let` writes its global
            // data slot rather than a local.
            if let Some(global) = module_global_name(&expr.span, cx) {
                return Ok(HirAssignTarget::Global { name: global });
            }
            Ok(HirAssignTarget::Ident {
                name: name.clone(),
                span: expr.span.clone(),
            })
        }
        // `self` as a bare target is the method receiver local; it binds
        // by the fixed name `self`, the same key MIR lowering uses to
        // look it up. Reassigning the receiver itself is rare but valid.
        ExprKind::SelfLower => Ok(HirAssignTarget::Ident {
            name: "self".to_string(),
            span: expr.span.clone(),
        }),
        ExprKind::Field { receiver, name } => {
            let r = lower_expr(receiver, &Ty::Error, cx)?;
            Ok(HirAssignTarget::Field {
                recv: r,
                name: name.clone(),
            })
        }
        ExprKind::Index { receiver, index } => {
            let r = lower_expr(receiver, &Ty::Error, cx)?;
            let i = lower_expr(index, &Ty::Int, cx)?;
            Ok(HirAssignTarget::Index { recv: r, index: i })
        }
        _ => Err(super::ty_error("invalid assignment target", &expr.span)),
    }
}

/// True when a call's callee is the built-in `print`: a bare identifier
/// of that name that the resolver left unbound (so it reaches the
/// compiler's print intrinsic rather than an imported `std/io`
/// function). The built-in form accepts any `ToString` value; an
/// imported `print` keeps its own String-typed signature.
fn is_builtin_print(callee: &Expr, cx: &LowerCtx<'_>) -> bool {
    let ExprKind::Ident { name, .. } = &callee.kind else {
        return false;
    };
    name == "print" && cx.fn_name_at(&callee.span).is_none()
}

/// Whether a method call's receiver is a bare type reference, marking the
/// call as an associated function call (`Type.func(args)`). Mirrors the
/// type checker's `type_ref_receiver`: an `Ident` bound to a struct or
/// enum declaration, or an unbound built-in type name.
fn is_type_ref_receiver(receiver: &Expr, cx: &LowerCtx<'_>) -> bool {
    use crate::resolve::Binding;
    let ExprKind::Ident { name, .. } = &receiver.kind else {
        return false;
    };
    match cx.resolved.map.lookup(&receiver.span) {
        Some(Binding::Struct(_)) | Some(Binding::Enum(_)) => true,
        Some(_) => false,
        None => matches!(
            name.as_str(),
            "Int" | "Float" | "Bool" | "String" | "Char" | "Unit" | "Array" | "List" | "Vec"
        ),
    }
}

/// Rewrite a bare-identifier match pattern into a unit-variant
/// constructor when the scrutinee is a user enum that declares a unit
/// variant by that name. The resolver binds every bare pattern ident as
/// a fresh binding; only the scrutinee type (known here) disambiguates a
/// nullary variant from a binding. The type checker performs the same
/// reconciliation, so this keeps HIR and tycheck in step.
fn reclassify_unit_variant(pat: &mut HirPattern, scrut_ty: &Ty, cx: &LowerCtx<'_>) {
    let HirPatternKind::Binding(name) = &pat.kind else {
        return;
    };
    let Ty::Enum { name: ename, .. } = scrut_ty.strip_self() else {
        return;
    };
    let Some(decl) = cx.env.enums.values().find(|e| &e.name == ename) else {
        return;
    };
    let is_unit_variant = decl.variants.iter().any(|v| {
        v.name == *name && matches!(v.payload, crate::tycheck::env::VariantPayloadSig::Unit)
    });
    if is_unit_variant {
        pat.kind = HirPatternKind::Constructor {
            name: Some(name.clone()),
            elements: Vec::new(),
        };
    }
}

/// If `receiver` is a bare enum name and `name` is one of its variants,
/// return that variant's index in declaration order. Returns `None` for
/// any other receiver (an ordinary struct field access, for example), so
/// the caller leaves that path untouched.
fn enum_variant_index(receiver: &Expr, name: &str, cx: &LowerCtx<'_>) -> Option<usize> {
    use crate::resolve::Binding;
    let ExprKind::Ident { .. } = &receiver.kind else {
        return None;
    };
    let Some(Binding::Enum(id)) = cx.resolved.map.lookup(&receiver.span) else {
        return None;
    };
    let sig = cx.env.enums.get(id)?;
    sig.variant(name).map(|(idx, _)| idx)
}

/// When the identifier at `span` resolves to a module-level `const` whose
/// initializer folds to a constant, return that value as an `HirExprKind` to
/// inline at the use site. Returns `None` for any other binding. A `const` is
/// always inlined; a module-level `let` is a mutable global with storage,
/// handled by [`module_global_name`].
fn module_const_literal(span: &Span, cx: &LowerCtx<'_>) -> Option<HirExprKind> {
    use crate::ast::DeclKind;
    use crate::resolve::Binding;
    let decl_id = match cx.resolved.map.lookup(span)? {
        Binding::Const(id) => *id,
        _ => return None,
    };
    let decl = cx.resolved.file.items.get(decl_id.0)?;
    let DeclKind::Const(c) = &decl.kind else {
        return None;
    };
    literal_hir_kind(&c.value)
}

/// When the identifier at `span` resolves to a mutable module-level `let`
/// (a `Binding::Static`), return the global's mangled symbol name so the
/// reference can read its data slot. Returns `None` for any other binding.
fn module_global_name(span: &Span, cx: &LowerCtx<'_>) -> Option<String> {
    use crate::ast::DeclKind;
    use crate::resolve::Binding;
    let decl_id = match cx.resolved.map.lookup(span)? {
        Binding::Static(id) => *id,
        _ => return None,
    };
    let decl = cx.resolved.file.items.get(decl_id.0)?;
    let DeclKind::Let(l) = &decl.kind else {
        return None;
    };
    Some(global_symbol(&l.name))
}

/// The link-time symbol name of a module-level global named `name`. Prefixed
/// so it cannot collide with a function symbol.
pub(crate) fn global_symbol(name: &str) -> String {
    format!("__raven_global_{name}")
}

/// Whether `e` folds to a compile-time constant. Used to validate a
/// module-level `const`/`let` initializer (which is inlined at each use site)
/// before lowering, so a non-constant initializer is a clear error rather
/// than a code-generation failure.
pub(crate) fn const_fold_kind(e: &Expr) -> Option<HirExprKind> {
    literal_hir_kind(e)
}

/// Fold a constant expression to its literal `HirExprKind`, or `None` when
/// `e` is not a compile-time constant. Handles literals, parentheses, unary
/// `-`/`!`, and binary arithmetic, comparison, bitwise, and boolean operators
/// over folded operands, so a `const` initializer such as `2 + 3` or
/// `60 * 60` inlines to its value at each use site. An operation that would
/// overflow or divide by zero does not fold (returns `None`), leaving the
/// error to surface elsewhere rather than panicking the compiler.
fn literal_hir_kind(e: &Expr) -> Option<HirExprKind> {
    match &e.kind {
        ExprKind::Int(i) => Some(HirExprKind::Int(*i)),
        ExprKind::Float(f) => Some(HirExprKind::Float(*f)),
        ExprKind::Bool(b) => Some(HirExprKind::Bool(*b)),
        ExprKind::Char(c) => Some(HirExprKind::Char(*c)),
        ExprKind::Str(s) | ExprKind::BlockStr(s) => Some(HirExprKind::Str(s.clone())),
        ExprKind::Paren(inner) => literal_hir_kind(inner),
        ExprKind::Unary { op, operand } => {
            let v = literal_hir_kind(operand)?;
            match (op, v) {
                (UnaryOp::Neg, HirExprKind::Int(i)) => i.checked_neg().map(HirExprKind::Int),
                (UnaryOp::Neg, HirExprKind::Float(f)) => Some(HirExprKind::Float(-f)),
                (UnaryOp::Not, HirExprKind::Bool(b)) => Some(HirExprKind::Bool(!b)),
                _ => None,
            }
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let l = literal_hir_kind(lhs)?;
            let r = literal_hir_kind(rhs)?;
            fold_binary(*op, l, r)
        }
        _ => None,
    }
}

/// Fold a binary operation over two already-folded literal operands. Integer
/// arithmetic is checked (overflow and divide-by-zero yield `None`).
fn fold_binary(op: BinaryOp, l: HirExprKind, r: HirExprKind) -> Option<HirExprKind> {
    use BinaryOp::*;
    use HirExprKind::{Bool, Float, Int};
    match (l, r) {
        (Int(a), Int(b)) => match op {
            Add => a.checked_add(b).map(Int),
            Sub => a.checked_sub(b).map(Int),
            Mul => a.checked_mul(b).map(Int),
            Div => a.checked_div(b).map(Int),
            Mod => a.checked_rem(b).map(Int),
            BitAnd => Some(Int(a & b)),
            BitOr => Some(Int(a | b)),
            BitXor => Some(Int(a ^ b)),
            Shl => u32::try_from(b)
                .ok()
                .and_then(|s| a.checked_shl(s))
                .map(Int),
            Shr => u32::try_from(b)
                .ok()
                .and_then(|s| a.checked_shr(s))
                .map(Int),
            Eq => Some(Bool(a == b)),
            Ne => Some(Bool(a != b)),
            Lt => Some(Bool(a < b)),
            Le => Some(Bool(a <= b)),
            Gt => Some(Bool(a > b)),
            Ge => Some(Bool(a >= b)),
            And | Or => None,
        },
        (Float(a), Float(b)) => match op {
            Add => Some(Float(a + b)),
            Sub => Some(Float(a - b)),
            Mul => Some(Float(a * b)),
            Div => Some(Float(a / b)),
            Eq => Some(Bool(a == b)),
            Ne => Some(Bool(a != b)),
            Lt => Some(Bool(a < b)),
            Le => Some(Bool(a <= b)),
            Gt => Some(Bool(a > b)),
            Ge => Some(Bool(a >= b)),
            _ => None,
        },
        (Bool(a), Bool(b)) => match op {
            And => Some(Bool(a && b)),
            Or => Some(Bool(a || b)),
            Eq => Some(Bool(a == b)),
            Ne => Some(Bool(a != b)),
            _ => None,
        },
        _ => None,
    }
}

/// When `receiver.name` is a `module.func` call through a stdlib import
/// alias (`import std/fs` then `fs.write(...)`), return the namespaced
/// function symbol (`std.<module>.<func>`) the call should target. The type
/// checker has already verified the call against this function's signature.
fn module_qualified_fn(receiver: &Expr, name: &str, cx: &LowerCtx<'_>) -> Option<String> {
    use crate::resolve::Binding;
    let ExprKind::Ident { generics, .. } = &receiver.kind else {
        return None;
    };
    if !generics.is_empty() {
        return None;
    }
    let Some(Binding::ImportAlias(import_id)) = cx.resolved.map.lookup(&receiver.span) else {
        return None;
    };
    let import = cx.resolved.map.imports.get(import_id.0)?;
    // The expander merged this module's functions under `mangled_prefix`, so an
    // alias call `alias.fn()` targets `<prefix>.fn`, for std, local, and
    // external sources alike.
    let prefix = import.mangled_prefix.as_ref()?;
    Some(format!(
        "{}{}{}",
        prefix,
        crate::resolve::stdlib::NAMESPACE_SEP,
        name
    ))
}

/// Wrap an interpolation part in a `to_string()` method call when its
/// type is neither a `String` nor one of the built-in scalars that have
/// a dedicated runtime rendering. The type checker has already verified
/// such a part implements `ToString`. A scalar or `String` part is left
/// as is so MIR keeps the allocation-light per-type conversion path.
fn to_string_if_needed(part: HirExpr) -> HirExpr {
    let scalar = matches!(
        part.ty.strip_self(),
        Ty::Str | Ty::Int | Ty::Bool | Ty::Float | Ty::Char | Ty::Error
    );
    if scalar {
        return part;
    }
    let span = part.span.clone();
    HirExpr {
        kind: HirExprKind::MethodCall {
            receiver: Box::new(part),
            name: "to_string".into(),
            args: Vec::new(),
        },
        ty: Ty::Str,
        span,
    }
}
