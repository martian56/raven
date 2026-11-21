//! Turn self-recursive returns into loops.
//!
//! `return f(args)` becomes "assign the parameters, loop again", and
//! `return x + f(args)` (or `*`) for an `int` result also folds `x` into an
//! accumulator, so `fib(n - 1) + fib(n - 2)` makes one call per frame. Only
//! functions where this is plainly safe are touched: no lambdas, loops,
//! `defer`, or `spawn` in the body, no local shadowing the function's name,
//! and a body that ends in `return`.

use crate::ast::{
    AssignOp, BinaryOp, Block, Decl, DeclKind, Expr, ExprKind, FunctionBody, Stmt, StmtKind,
    TypeKind,
};
use crate::span::Span;

pub fn rewrite(decl: &mut Decl) {
    let DeclKind::Function(f) = &mut decl.kind else {
        return;
    };
    if !f.generics.is_empty() || f.params.is_empty() {
        return;
    }
    let FunctionBody::Block(body) = &f.body else {
        return;
    };
    if body.trailing.is_some()
        || !matches!(
            body.stmts.last().map(|s| &s.kind),
            Some(StmtKind::Return(Some(_)))
        )
    {
        return;
    }
    let locals = crate::resolve::sugar::local_names(f);
    let params: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
    if locals.contains(&f.name) || !body_is_simple(decl) {
        return;
    }
    let DeclKind::Function(f) = &mut decl.kind else {
        return;
    };
    let int_result = matches!(
        f.ret.as_ref().map(|t| &t.kind),
        Some(TypeKind::Path(p)) if p.segments.len() == 1 && matches!(p.segments[0].name.as_str(), "int" | "Int")
    );
    let name = f.name.clone();
    let FunctionBody::Block(body) = &mut f.body else {
        return;
    };
    let Some(op) = choose_op(body, &name, params.len(), int_result) else {
        return;
    };
    let span = body.span.clone();
    let acc = "__tailrec_acc".to_string();
    let mut counter = 0usize;
    rewrite_block(body, &name, &params, op, &acc, &mut counter);
    let stmts = std::mem::take(&mut body.stmts);
    let loop_expr = expr(
        ExprKind::Loop(Block {
            stmts,
            trailing: None,
            span: fresh(&span),
        }),
        &span,
    );
    let mut new = Vec::new();
    if let Some(op) = op {
        let identity = if op == BinaryOp::Add { 0 } else { 1 };
        new.push(stmt(
            StmtKind::Let {
                name: acc.clone(),
                ty: None,
                init: Some(expr(ExprKind::Int(identity), &span)),
                mutable: true,
            },
            &span,
        ));
    }
    new.push(stmt(StmtKind::Expr(loop_expr), &span));
    // The loop only leaves through `return`; this marks the end unreachable
    // so no fallback return of the wrong type is generated after it.
    new.push(stmt(
        StmtKind::Expr(expr(
            ExprKind::Call {
                callee: Box::new(ident("panic", &span)),
                args: vec![expr(ExprKind::Str("unreachable".to_string()), &span)],
            },
            &span,
        )),
        &span,
    ));
    body.stmts = new;
}

/// `None` when no return is a self call; `Some(None)` for plain tail calls
/// only; `Some(Some(op))` when an accumulated `+` or `*` is used.
fn choose_op(body: &Block, name: &str, arity: usize, int_result: bool) -> Option<Option<BinaryOp>> {
    let mut found = false;
    let mut op: Option<BinaryOp> = None;
    let mut conflict = false;
    each_return(body, &mut |e: &Expr| match classify(e, name, arity) {
        Some(Site::Tail) => found = true,
        Some(Site::Acc(o)) if int_result => {
            found = true;
            match op {
                None => op = Some(o),
                Some(prev) if prev != o => conflict = true,
                _ => {}
            }
        }
        _ => {}
    });
    if !found || conflict {
        return None;
    }
    Some(op)
}

enum Site {
    Tail,
    Acc(BinaryOp),
}

fn is_self_call(e: &Expr, name: &str, arity: usize) -> bool {
    match &e.kind {
        ExprKind::Call { callee, args } => {
            matches!(&callee.kind, ExprKind::Ident { name: n, generics } if n == name && generics.is_empty())
                && args.len() == arity
                && !args
                    .iter()
                    .any(|a| matches!(a.kind, ExprKind::NamedArg { .. }))
        }
        _ => false,
    }
}

fn classify(e: &Expr, name: &str, arity: usize) -> Option<Site> {
    if is_self_call(e, name, arity) {
        return Some(Site::Tail);
    }
    if let ExprKind::Binary { op, rhs, .. } = &e.kind {
        if matches!(op, BinaryOp::Add | BinaryOp::Mul) && is_self_call(rhs, name, arity) {
            return Some(Site::Acc(*op));
        }
    }
    None
}

fn each_return(b: &Block, f: &mut dyn FnMut(&Expr)) {
    for s in &b.stmts {
        match &s.kind {
            StmtKind::Return(Some(e)) => f(e),
            StmtKind::Expr(e) => each_return_expr(e, f),
            _ => {}
        }
    }
}

fn each_return_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    match &e.kind {
        ExprKind::Block(b) => each_return(b, f),
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            each_return(then_branch, f);
            if let Some(eb) = else_branch {
                match &**eb {
                    crate::ast::ElseBranch::If(x) => each_return_expr(x, f),
                    crate::ast::ElseBranch::Block(b) => each_return(b, f),
                }
            }
        }
        ExprKind::Match { arms, .. } => {
            for a in arms {
                each_return_expr(&a.body, f);
            }
        }
        _ => {}
    }
}

/// Whether every return can be rewritten in place: no construct that
/// would capture a parameter or catch the inserted `continue`.
fn body_is_simple(decl: &mut Decl) -> bool {
    let ok = std::cell::Cell::new(true);
    crate::ast::visit::walk_decl(
        decl,
        &mut |e: &mut Expr| {
            if matches!(
                e.kind,
                ExprKind::Lambda { .. }
                    | ExprKind::Loop(_)
                    | ExprKind::While { .. }
                    | ExprKind::For { .. }
                    | ExprKind::WhileLet { .. }
                    | ExprKind::Comprehension { .. }
            ) {
                ok.set(false);
            }
        },
        &mut |b: &mut Block| {
            if b.stmts.iter().any(|s| {
                matches!(
                    s.kind,
                    StmtKind::Defer(_)
                        | StmtKind::Spawn(_)
                        | StmtKind::Break(_)
                        | StmtKind::Continue
                )
            }) {
                ok.set(false);
            }
        },
    );
    ok.get()
}

fn rewrite_block(
    b: &mut Block,
    name: &str,
    params: &[String],
    op: Option<BinaryOp>,
    acc: &str,
    counter: &mut usize,
) {
    for s in &mut b.stmts {
        match &mut s.kind {
            StmtKind::Return(Some(e)) => {
                let span = s.span.clone();
                let taken = std::mem::replace(e, expr(ExprKind::Int(0), &span));
                s.kind = rewrite_return(taken, name, params, op, acc, counter, &span);
            }
            StmtKind::Expr(e) => rewrite_expr(e, name, params, op, acc, counter),
            _ => {}
        }
    }
}

fn rewrite_expr(
    e: &mut Expr,
    name: &str,
    params: &[String],
    op: Option<BinaryOp>,
    acc: &str,
    counter: &mut usize,
) {
    match &mut e.kind {
        ExprKind::Block(b) => rewrite_block(b, name, params, op, acc, counter),
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            rewrite_block(then_branch, name, params, op, acc, counter);
            if let Some(eb) = else_branch {
                match &mut **eb {
                    crate::ast::ElseBranch::If(x) => {
                        rewrite_expr(x, name, params, op, acc, counter)
                    }
                    crate::ast::ElseBranch::Block(b) => {
                        rewrite_block(b, name, params, op, acc, counter)
                    }
                }
            }
        }
        ExprKind::Match { arms, .. } => {
            for a in arms {
                rewrite_expr(&mut a.body, name, params, op, acc, counter);
            }
        }
        _ => {}
    }
}

fn rewrite_return(
    e: Expr,
    name: &str,
    params: &[String],
    op: Option<BinaryOp>,
    acc: &str,
    counter: &mut usize,
    span: &Span,
) -> StmtKind {
    let site = classify(&e, name, params.len());
    let (prefix, call) = match (site, op) {
        (Some(Site::Tail), _) => (None, e),
        (Some(Site::Acc(o)), Some(a)) if o == a => match e.kind {
            ExprKind::Binary { lhs, rhs, .. } => (Some(*lhs), *rhs),
            _ => unreachable!("classified as a binary site"),
        },
        _ => {
            let value = match op {
                Some(a) => binary(a, ident(acc, span), e, span),
                None => e,
            };
            return StmtKind::Return(Some(value));
        }
    };
    let ExprKind::Call { args, .. } = call.kind else {
        unreachable!("classified as a self call");
    };
    let mut stmts = Vec::new();
    if let (Some(x), Some(a)) = (prefix, op) {
        stmts.push(stmt(
            StmtKind::Assign {
                target: ident(acc, span),
                op: AssignOp::Assign,
                value: binary(a, ident(acc, span), x, span),
            },
            span,
        ));
    }
    let mut temps = Vec::new();
    for a in args {
        *counter += 1;
        let t = format!("__tailrec_arg{}", counter);
        stmts.push(stmt(
            StmtKind::Let {
                name: t.clone(),
                ty: None,
                init: Some(a),
                mutable: false,
            },
            span,
        ));
        temps.push(t);
    }
    for (p, t) in params.iter().zip(&temps) {
        stmts.push(stmt(
            StmtKind::Assign {
                target: ident(p, span),
                op: AssignOp::Assign,
                value: ident(t, span),
            },
            span,
        ));
    }
    stmts.push(stmt(StmtKind::Continue, span));
    StmtKind::Expr(expr(
        ExprKind::Block(Block {
            stmts,
            trailing: None,
            span: fresh(span),
        }),
        span,
    ))
}

fn fresh(span: &Span) -> Span {
    span.in_copy(crate::span::next_copy_id())
}

fn expr(kind: ExprKind, span: &Span) -> Expr {
    Expr {
        kind,
        span: fresh(span),
    }
}

fn stmt(kind: StmtKind, span: &Span) -> Stmt {
    Stmt {
        kind,
        span: fresh(span),
    }
}

fn ident(name: &str, span: &Span) -> Expr {
    expr(
        ExprKind::Ident {
            name: name.to_string(),
            generics: Vec::new(),
        },
        span,
    )
}

fn binary(op: BinaryOp, lhs: Expr, rhs: Expr, span: &Span) -> Expr {
    expr(
        ExprKind::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        span,
    )
}
