//! Closure capture analysis and lambda-body lifting for MIR lowering.
//!
//! A lambda expression becomes two things in MIR:
//!
//! * a `ClosureCreate` rvalue at the definition site that allocates a
//!   `Closure` object, stores the lifted body's function pointer, and
//!   copies each captured value into the capture environment, and
//! * a standalone, top-level `MirFunction` (the "lifted body") whose
//!   leading parameter is the capture environment and whose remaining
//!   parameters are the lambda's own parameters.
//!
//! Capture analysis walks the lambda body and collects every free
//! variable: an identifier that resolves to a local or parameter of an
//! enclosing function (one that is in scope at the definition site) and
//! is neither a lambda parameter nor a binding introduced inside the
//! lambda body. Top-level functions and constants are referenced by
//! symbol, not captured.
//!
//! Captures are by value: the value is copied into the environment at
//! closure-creation time. For a GC-managed value the copied value is the
//! same pointer, so the captured object aliases the original and
//! mutations through the heap object stay visible.
//!
//! See `docs/v2/specs/codegen.md` and `docs/v2/specs/object-layout.md`.

use std::collections::HashSet;

use crate::hir::expr::{HirBlock, HirExpr, HirExprKind, InterpolPart};
use crate::hir::stmt::{HirAssignTarget, HirStmt, HirStmtKind};
use crate::hir::ty::HirTy;

use super::super::ir::{MirFunction, MirLocal, MirRvalue, MirTerminator};
use super::super::ty::{MirFfiTy, MirType};
use super::{mir_ty, LowerCx, Scope, SubstMap};
use crate::codegen::layout::is_gc_pointer;
use crate::resolve::DeclId;

/// One captured variable: its source name, its MIR type, and the
/// enclosing-scope local that currently holds the value to copy.
pub struct Capture {
    pub name: String,
    pub ty: MirType,
    pub source: MirLocal,
}

/// The MIR type used for a lifted body's leading env parameter: a raw
/// pointer-width value the GC does not trace (the capture environment is
/// owned by the `Closure` object and traced through its descriptor).
pub fn env_param_ty() -> MirType {
    MirType::Ffi(MirFfiTy::CSize)
}

/// Lower a lambda expression. Returns the closure-create rvalue plus the
/// lifted function name. The lifted body is pushed onto `cx.lifted`.
pub fn lower_lambda(
    cx: &mut LowerCx<'_>,
    params: &[(String, HirTy, crate::span::Span)],
    ret: &HirTy,
    body: &HirBlock,
) -> MirRvalue {
    // Names bound by the lambda's own parameters never count as captures.
    let mut bound: HashSet<String> = params.iter().map(|(n, _, _)| n.clone()).collect();
    // Collect free identifier names referenced by the body.
    let mut free: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    collect_free_block(body, &mut bound, &mut seen, &mut free);

    // A free name is a capture only when it resolves to a local or
    // parameter that is in scope at the definition site. Names that do
    // not resolve in the enclosing scopes are references to top-level
    // functions or constants, which codegen addresses by symbol.
    let mut captures: Vec<Capture> = Vec::new();
    for name in free {
        if let Some(local) = cx.lookup(&name) {
            let ty = cx.builder.locals()[local.0 as usize].ty.clone();
            captures.push(Capture {
                name,
                ty,
                source: local,
            });
        }
    }

    // Order GC-pointer captures first so the runtime's contract that the
    // leading `capture_ptr_count` slots are traced GC pointers holds.
    captures.sort_by_key(|c| !is_gc_pointer(&c.ty));

    let lifted_name = mint_lifted_name(cx);
    let ret_ty = mir_ty(ret, cx.subst);

    // Build the lifted body as a standalone function. Any lambdas nested
    // inside the body lift their own bodies; surface those too. The
    // lifted body may also call generic functions: those call sites are
    // collected during its lowering and must reach the monomorphization
    // worklist exactly as an ordinary function body's calls do, otherwise
    // a generic function reachable only through the closure is never
    // instantiated.
    let LiftedBody {
        func: lifted,
        nested,
        pending,
    } = lift_body(cx, &lifted_name, params, ret_ty, body, &captures);
    cx.lifted.push(lifted);
    cx.lifted.extend(nested);
    cx.pending_calls.extend(pending);

    let capture_ops = captures
        .iter()
        .map(|c| super::super::ir::MirOperand::Copy(c.source))
        .collect();
    let capture_tys = captures.iter().map(|c| c.ty.clone()).collect();

    MirRvalue::ClosureCreate {
        fn_name: lifted_name,
        captures: capture_ops,
        capture_tys,
    }
}

/// Mint a globally unique name for a lifted closure body.
fn mint_lifted_name(cx: &mut LowerCx<'_>) -> String {
    let n = cx.lambda_seq;
    cx.lambda_seq += 1;
    format!("{}$closure${}", cx.enclosing, n)
}

/// The result of lifting one lambda body: the standalone `MirFunction`,
/// any functions lifted from lambdas nested inside it, and the generic
/// call sites discovered while lowering the body. The pending calls are
/// surfaced so the monomorphizer queues every generic instantiation the
/// closure reaches, just as it does for an ordinary function body.
struct LiftedBody {
    func: MirFunction,
    nested: Vec<MirFunction>,
    pending: Vec<(DeclId, SubstMap)>,
}

/// Lift a lambda body into a standalone `MirFunction`.
///
/// The lifted function's parameters are the capture environment pointer
/// first, then the lambda's declared parameters. The body opens by
/// reading each captured value from the environment into a fresh local
/// bound under the capture's source name, so the body's identifier
/// references resolve to those locals exactly as they did at the
/// definition site.
fn lift_body(
    cx: &LowerCx<'_>,
    name: &str,
    params: &[(String, HirTy, crate::span::Span)],
    ret_ty: MirType,
    body: &HirBlock,
    captures: &[Capture],
) -> LiftedBody {
    use super::super::builder::FunctionBuilder;

    let mut builder = FunctionBuilder::new(
        name.to_string(),
        name.to_string(),
        ret_ty.clone(),
        body.span.clone(),
    );

    // Leading env parameter: a raw pointer-width value.
    let env_local = builder.add_param("__env".to_string(), env_param_ty());

    // The lambda's declared parameters follow the env parameter.
    let mut param_scope = Scope::new();
    for (pname, pty, _) in params {
        let mty = mir_ty(pty, cx.subst);
        let local = builder.add_param(pname.clone(), mty);
        param_scope.insert(pname.clone(), local);
    }

    let entry = builder.new_block();
    let mut body_cx = LowerCx {
        builder,
        current: entry,
        subst: cx.subst,
        scopes: vec![param_scope],
        loops: Vec::new(),
        pending_calls: Vec::new(),
        decls: cx.decls,
        diverged: false,
        lifted: Vec::new(),
        lambda_seq: 0,
        enclosing: name.to_string(),
    };

    // Read each capture from the env into a local bound under its source
    // name. The body then references captures as ordinary locals.
    for (slot, cap) in captures.iter().enumerate() {
        let local = body_cx
            .builder
            .named_local(cap.name.clone(), cap.ty.clone());
        body_cx.builder.assign(
            body_cx.current,
            local,
            MirRvalue::EnvLoad {
                env: super::super::ir::MirOperand::Copy(env_local),
                slot,
                ty: cap.ty.clone(),
            },
        );
        body_cx.bind(cap.name.clone(), local);
    }

    let result = super::stmt::lower_block(&mut body_cx, body);

    if !body_cx.builder.is_closed(body_cx.current) {
        if body_cx.diverged && body_cx.builder.is_empty_open(body_cx.current) {
            body_cx
                .builder
                .close_block(body_cx.current, MirTerminator::Unreachable);
        } else {
            body_cx
                .builder
                .close_block(body_cx.current, MirTerminator::Return(result));
        }
    }

    // A lambda body may itself contain nested lambdas; surface their
    // lifted functions through the enclosing function's accumulator.
    let nested = std::mem::take(&mut body_cx.lifted);
    // The body's own generic call sites travel up to the enclosing
    // function so the monomorphizer instantiates every callee the
    // closure reaches. Without this they would be dropped with `body_cx`.
    let pending = std::mem::take(&mut body_cx.pending_calls);
    LiftedBody {
        func: body_cx.builder.finish(entry),
        nested,
        pending,
    }
}

// ----- free-variable collection -----

fn collect_free_block(
    block: &HirBlock,
    bound: &mut HashSet<String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<String>,
) {
    // A block introduces its own bindings; names bound inside shadow the
    // enclosing scope for the rest of the block. Track them so they are
    // not mistaken for captures, and restore the set on exit.
    let snapshot: Vec<String> = bound.iter().cloned().collect();
    for stmt in &block.stmts {
        collect_free_stmt(stmt, bound, seen, out);
    }
    if let Some(tail) = &block.tail {
        collect_free_expr(tail, bound, seen, out);
    }
    *bound = snapshot.into_iter().collect();
}

fn collect_free_stmt(
    stmt: &HirStmt,
    bound: &mut HashSet<String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<String>,
) {
    match &stmt.kind {
        HirStmtKind::Let { name, init, .. } => {
            collect_free_expr(init, bound, seen, out);
            // The binding is visible after its initializer.
            bound.insert(name.clone());
        }
        HirStmtKind::Expr(e) => collect_free_expr(e, bound, seen, out),
        HirStmtKind::Assign { target, value } => {
            match target {
                HirAssignTarget::Ident { name, .. } => record_use(name, bound, seen, out),
                HirAssignTarget::Field { recv, .. } => collect_free_expr(recv, bound, seen, out),
                HirAssignTarget::Index { recv, index } => {
                    collect_free_expr(recv, bound, seen, out);
                    collect_free_expr(index, bound, seen, out);
                }
            }
            collect_free_expr(value, bound, seen, out);
        }
        HirStmtKind::Defer(e) | HirStmtKind::Spawn(e) => collect_free_expr(e, bound, seen, out),
    }
}

fn record_use(
    name: &str,
    bound: &HashSet<String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<String>,
) {
    if bound.contains(name) {
        return;
    }
    if seen.insert(name.to_string()) {
        out.push(name.to_string());
    }
}

fn collect_free_expr(
    expr: &HirExpr,
    bound: &mut HashSet<String>,
    seen: &mut HashSet<String>,
    out: &mut Vec<String>,
) {
    match &expr.kind {
        HirExprKind::Int(_)
        | HirExprKind::Float(_)
        | HirExprKind::Bool(_)
        | HirExprKind::Str(_)
        | HirExprKind::Char(_)
        | HirExprKind::CStr(_)
        | HirExprKind::Unit
        | HirExprKind::SelfValue
        | HirExprKind::NoneCtor
        | HirExprKind::TypeName(_)
        | HirExprKind::FieldNames(_)
        | HirExprKind::Continue => {}
        HirExprKind::Ident(name) => record_use(name, bound, seen, out),
        HirExprKind::Array(items) => {
            for it in items {
                collect_free_expr(it, bound, seen, out);
            }
        }
        HirExprKind::StructLit { fields, .. } => {
            for (_, e) in fields {
                collect_free_expr(e, bound, seen, out);
            }
        }
        HirExprKind::Paren(inner)
        | HirExprKind::IterNew(inner)
        | HirExprKind::IterNext(inner)
        | HirExprKind::OkCtor(inner)
        | HirExprKind::ErrCtor(inner)
        | HirExprKind::SomeCtor(inner) => collect_free_expr(inner, bound, seen, out),
        HirExprKind::Block(b) => collect_free_block(b, bound, seen, out),
        HirExprKind::Unary { operand, .. } => collect_free_expr(operand, bound, seen, out),
        HirExprKind::Binary { lhs, rhs, .. } => {
            collect_free_expr(lhs, bound, seen, out);
            collect_free_expr(rhs, bound, seen, out);
        }
        HirExprKind::Call { callee, args, .. } => {
            collect_free_expr(callee, bound, seen, out);
            for a in args {
                collect_free_expr(a, bound, seen, out);
            }
        }
        HirExprKind::EnumCreate { args, .. } => {
            for a in args {
                collect_free_expr(a, bound, seen, out);
            }
        }
        HirExprKind::PtrBuiltin { args, .. } => {
            for a in args {
                collect_free_expr(a, bound, seen, out);
            }
        }
        HirExprKind::MethodCall { receiver, args, .. } => {
            collect_free_expr(receiver, bound, seen, out);
            for a in args {
                collect_free_expr(a, bound, seen, out);
            }
        }
        HirExprKind::AssocCall { args, .. } => {
            for a in args {
                collect_free_expr(a, bound, seen, out);
            }
        }
        HirExprKind::Field { receiver, .. } => collect_free_expr(receiver, bound, seen, out),
        HirExprKind::Index { receiver, index } => {
            collect_free_expr(receiver, bound, seen, out);
            collect_free_expr(index, bound, seen, out);
        }
        HirExprKind::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_free_expr(cond, bound, seen, out);
            collect_free_block(then_block, bound, seen, out);
            if let Some(e) = else_block {
                collect_free_block(e, bound, seen, out);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            collect_free_expr(scrutinee, bound, seen, out);
            for arm in arms {
                // Pattern bindings are local to the arm.
                let snapshot: Vec<String> = bound.iter().cloned().collect();
                bind_pattern_names(&arm.pattern, bound);
                if let Some(g) = &arm.guard {
                    collect_free_expr(g, bound, seen, out);
                }
                collect_free_expr(&arm.body, bound, seen, out);
                *bound = snapshot.into_iter().collect();
            }
        }
        HirExprKind::Loop(b) => collect_free_block(b, bound, seen, out),
        HirExprKind::While { cond, body } => {
            collect_free_expr(cond, bound, seen, out);
            collect_free_block(body, bound, seen, out);
        }
        HirExprKind::Return(v) | HirExprKind::Break(v) => {
            if let Some(e) = v {
                collect_free_expr(e, bound, seen, out);
            }
        }
        HirExprKind::Interpolate(parts) => {
            for p in parts {
                if let InterpolPart::Expr(e) = p {
                    collect_free_expr(e, bound, seen, out);
                }
            }
        }
        HirExprKind::RangeNew { start, end, .. } => {
            collect_free_expr(start, bound, seen, out);
            collect_free_expr(end, bound, seen, out);
        }
        HirExprKind::Lambda { params, body, .. } => {
            // A nested lambda introduces its own parameter bindings; names
            // it captures from this lambda's scope are themselves free
            // variables of this lambda (transitive capture).
            let snapshot: Vec<String> = bound.iter().cloned().collect();
            for (pname, _, _) in params {
                bound.insert(pname.clone());
            }
            collect_free_block(body, bound, seen, out);
            *bound = snapshot.into_iter().collect();
        }
        HirExprKind::DynCoerce { value, .. } => collect_free_expr(value, bound, seen, out),
    }
}

/// Add every name a pattern introduces to the bound set.
fn bind_pattern_names(pat: &crate::hir::pattern::HirPattern, bound: &mut HashSet<String>) {
    use crate::hir::pattern::HirPatternKind;
    match &pat.kind {
        HirPatternKind::Wildcard | HirPatternKind::Literal(_) | HirPatternKind::Range { .. } => {}
        HirPatternKind::Binding(name) => {
            bound.insert(name.clone());
        }
        HirPatternKind::Constructor { elements, .. } => {
            for e in elements {
                bind_pattern_names(e, bound);
            }
        }
        HirPatternKind::Struct { fields, .. } => {
            for f in fields {
                if let Some(inner) = &f.pattern {
                    bind_pattern_names(inner, bound);
                } else {
                    bound.insert(f.name.clone());
                }
            }
        }
    }
}
