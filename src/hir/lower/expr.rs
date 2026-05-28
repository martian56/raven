//! Lowering AST expressions and blocks to HIR.

use crate::ast::{BinaryOp, Block as AstBlock, ElseBranch, Expr, ExprKind, LambdaBody, UnaryOp};
use crate::error::RavenError;
use crate::span::Span;
use crate::tycheck::Ty;

use crate::hir::expr::{
    HirArm, HirBinaryOp, HirBlock, HirExpr, HirExprKind, HirUnaryOp, InterpolPart,
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
    let kind = match &expr.kind {
        ExprKind::Int(i) => HirExprKind::Int(*i),
        ExprKind::Float(f) => HirExprKind::Float(*f),
        ExprKind::Bool(b) => HirExprKind::Bool(*b),
        ExprKind::Str(s) | ExprKind::BlockStr(s) => lower_string_literal(s, &span),
        ExprKind::CStr(s) => HirExprKind::CStr(s.clone()),
        ExprKind::Char(c) => HirExprKind::Char(*c),
        ExprKind::SelfLower => HirExprKind::SelfValue,
        ExprKind::SelfUpper => HirExprKind::Ident("Self".into()),
        ExprKind::Ident { name, .. } if name == "None" => HirExprKind::NoneCtor,
        ExprKind::Ident { name, .. } => HirExprKind::Ident(name.clone()),
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
            HirExprKind::Binary {
                op: lower_binop(*op),
                lhs: Box::new(l),
                rhs: Box::new(r),
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
            let c = lower_expr(callee, &Ty::Error, cx)?;
            let mut lowered = Vec::with_capacity(args.len());
            for a in args {
                lowered.push(lower_expr(a, &Ty::Error, cx)?);
            }
            HirExprKind::Call {
                callee: Box::new(c),
                args: lowered,
            }
        }
        ExprKind::MethodCall {
            receiver,
            name,
            args,
            ..
        } => {
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
        ExprKind::Field { receiver, name } => {
            let r = lower_expr(receiver, &Ty::Error, cx)?;
            HirExprKind::Field {
                receiver: Box::new(r),
                name: name.clone(),
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
            let mut lowered = Vec::with_capacity(arms.len());
            for arm in arms {
                let pattern = lower_pattern(&arm.pattern, cx)?;
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
    Ok(HirExpr { kind, ty, span })
}

fn lower_unop(op: UnaryOp) -> HirUnaryOp {
    match op {
        UnaryOp::Neg => HirUnaryOp::Neg,
        UnaryOp::Not => HirUnaryOp::Not,
        UnaryOp::Ref => HirUnaryOp::Ref,
    }
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

/// Inspect a raw string literal: if it contains `${...}` it is an
/// interpolation; otherwise it is a plain string.
fn lower_string_literal(s: &str, span: &Span) -> HirExprKind {
    if !s.contains("${") {
        return HirExprKind::Str(s.to_string());
    }
    let parts = split_interpolation(s, span);
    if parts.iter().all(|p| matches!(p, InterpolPart::Text(_))) {
        // No actual `${...}` once we look closely (e.g. `\$`).
        let mut buf = String::new();
        for p in &parts {
            if let InterpolPart::Text(t) = p {
                buf.push_str(t);
            }
        }
        HirExprKind::Str(buf)
    } else {
        HirExprKind::Interpolate(parts)
    }
}

/// Split the raw contents of a `StringLit` into text and expression
/// fragments. Embedded expressions are stored as their textual source
/// inside an `Expr` placeholder; the lowering pass cannot re-parse
/// them in isolation without re-running the full pipeline, so this
/// release ships them as raw `Ident` placeholders when the embedded
/// snippet parses as a bare identifier and as text otherwise. The
/// embedded form is documented in `docs/v2/specs/hir.md`.
fn split_interpolation(s: &str, span: &Span) -> Vec<InterpolPart> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Find matching `}`.
            let start = i + 2;
            let mut depth = 1;
            let mut j = start;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
                j += 1;
            }
            if depth == 0 && j < bytes.len() {
                if !text.is_empty() {
                    parts.push(InterpolPart::Text(std::mem::take(&mut text)));
                }
                let snippet = &s[start..j];
                parts.push(InterpolPart::Expr(make_snippet_expr(snippet, span)));
                i = j + 1;
                continue;
            }
            // Unterminated; fall through and treat as text.
            text.push(s[i..].chars().next().unwrap_or('$'));
            i += 1;
        } else {
            let c = s[i..].chars().next().unwrap_or(' ');
            text.push(c);
            i += c.len_utf8();
        }
    }
    if !text.is_empty() {
        parts.push(InterpolPart::Text(text));
    }
    parts
}

/// Wrap a literal snippet as an HIR `Ident` placeholder. The MIR pass
/// is responsible for performing the real string-conversion call; for
/// now we expose the snippet verbatim so snapshot tests can show what
/// fragments the splitter produced. The span of the surrounding string
/// is reused because the lexer keeps the literal as one token.
fn make_snippet_expr(snippet: &str, span: &Span) -> HirExpr {
    HirExpr {
        kind: HirExprKind::Ident(snippet.trim().to_string()),
        ty: Ty::Str,
        span: span.clone(),
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

/// Lower a `for pat in iter { body }` into:
///
/// ```text
/// {
///     let __iter = IterNew(iter);
///     loop {
///         match IterNext(__iter) {
///             Some(pat) => body,
///             None => break,
///         }
///     }
/// }
/// ```
fn lower_for(
    pat: &crate::ast::Pattern,
    iter: &Expr,
    body: &AstBlock,
    _result_ty: &Ty,
    span: &Span,
    cx: &LowerCtx<'_>,
) -> Result<HirExpr, RavenError> {
    let iter_expr = lower_expr(iter, &Ty::Error, cx)?;
    let element_ty = match &iter_expr.ty {
        Ty::List(t) => (**t).clone(),
        _ => Ty::Error,
    };
    let body_block = lower_block_to_block(body, &Ty::Unit, cx)?;
    let iter_name = cx.fresh("iter");
    let iter_ty = iter_expr.ty.clone();
    let iter_let = let_stmt(
        &iter_name,
        iter_ty.clone(),
        make_expr(
            HirExprKind::IterNew(Box::new(iter_expr)),
            iter_ty.clone(),
            span.clone(),
        ),
        span.clone(),
    );
    let iter_ref = ident_expr(&iter_name, iter_ty.clone(), span.clone());
    let next_expr = make_expr(
        HirExprKind::IterNext(Box::new(iter_ref)),
        Ty::Option(Box::new(element_ty.clone())),
        span.clone(),
    );

    let some_pat = HirPattern {
        kind: HirPatternKind::Constructor {
            name: Some("Some".to_string()),
            elements: vec![lower_pattern(pat, cx)?],
        },
        span: span.clone(),
    };
    let none_pat = HirPattern {
        kind: HirPatternKind::Constructor {
            name: Some("None".to_string()),
            elements: Vec::new(),
        },
        span: span.clone(),
    };
    let some_arm = HirArm {
        pattern: some_pat,
        guard: None,
        body: make_expr(HirExprKind::Block(body_block), Ty::Unit, span.clone()),
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
            scrutinee: Box::new(next_expr),
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
        stmts: vec![iter_let],
        tail: Some(Box::new(loop_expr)),
        ty: Ty::Unit,
        span: span.clone(),
    };
    Ok(make_expr(HirExprKind::Block(block), Ty::Unit, span.clone()))
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
        ExprKind::Ident { name, .. } => {
            // x op= v  ->  x = x op v
            let load = make_expr(
                HirExprKind::Ident(name.clone()),
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
            Ok(vec![assign_stmt(
                HirAssignTarget::Ident {
                    name: name.clone(),
                    span: target.span.clone(),
                },
                combined,
                span.clone(),
            )])
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
        ExprKind::Ident { name, .. } => Ok(HirAssignTarget::Ident {
            name: name.clone(),
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
