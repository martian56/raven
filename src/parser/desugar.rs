//! Lowers surface-only syntax to core AST after parsing: `class` bodies,
//! `if let`, `while let`, and `let ... else`. The formatter works on the
//! undesugared tree.

use crate::ast::visit;
use crate::ast::{
    BinaryOp, Block, Decl, DeclKind, ElseBranch, Expr, ExprKind, File, GenericParam, Impl,
    MatchArm, Pattern, PatternKind, Stmt, StmtKind, Type, TypeKind, TypePath, TypePathSegment,
    VariantPayload,
};
use crate::error::{ParseError, RavenError};
use crate::span::Span;

use super::merge_spans;

pub fn desugar(file: &mut File) -> Result<(), RavenError> {
    let mut items = Vec::with_capacity(file.items.len());
    for mut decl in std::mem::take(&mut file.items) {
        if let DeclKind::Enum(e) = &mut decl.kind {
            for v in &mut e.variants {
                if let VariantPayload::Struct(fields) = &mut v.payload {
                    let tys = std::mem::take(fields).into_iter().map(|f| f.ty).collect();
                    v.payload = VariantPayload::Tuple(tys);
                }
            }
        }
        let extra = split_class(&mut decl);
        items.push(decl);
        items.extend(extra);
    }
    let mut error = None;
    for decl in &mut items {
        visit::walk_decl(
            decl,
            &mut |e: &mut Expr| lower_expr(e),
            &mut |b: &mut Block| {
                if error.is_none() {
                    if let Err(e) = lower_let_tuple(b).and_then(|_| lower_let_else(b)) {
                        error = Some(e);
                    }
                }
            },
        );
    }
    let module_values: std::collections::HashSet<String> = items
        .iter()
        .filter_map(|d| match &d.kind {
            DeclKind::Let(l) => Some(l.name.clone()),
            DeclKind::Const(c) => Some(c.name.clone()),
            DeclKind::Function(f) if is_builtin_name(&f.name) => Some(f.name.clone()),
            _ => None,
        })
        .chain(items.iter().flat_map(|d| {
            match &d.kind {
                DeclKind::Import(i) => i
                    .selectors
                    .iter()
                    .map(|s| s.local().to_string())
                    .filter(|n| is_builtin_name(n))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            }
        }))
        .collect();
    for decl in &mut items {
        let mut each = |f: &mut crate::ast::Function| {
            let mut values = crate::resolve::sugar::local_names(f);
            values.extend(module_values.iter().cloned());
            visit::exprs_in_fn(f, &mut |e: &mut Expr| {
                index_not_type_args(e, &values);
                empty_collection_call(e, &values);
                builtin_call(e, &values);
            });
        };
        match &mut decl.kind {
            DeclKind::Function(f) => each(f),
            DeclKind::Impl(i) => i.items.iter_mut().for_each(&mut each),
            DeclKind::Trait(t) => t.members.iter_mut().for_each(&mut each),
            DeclKind::Struct(s) => s.methods.iter_mut().for_each(&mut each),
            _ => {}
        }
    }
    crate::resolve::sugar::distribute_class_methods(&mut items);
    if let Some(e) = error {
        return Err(e);
    }
    crate::resolve::sugar::expand_calls(&mut items, false)?;
    file.items = items;
    Ok(())
}

/// A `class` (or a `struct` with methods) becomes the struct, one inherent
/// impl with every method, and an empty impl per listed trait. Expansion
/// moves each trait's methods into its impl.
fn split_class(decl: &mut Decl) -> Vec<Decl> {
    let DeclKind::Struct(s) = &mut decl.kind else {
        return Vec::new();
    };
    if s.methods.is_empty() && s.implements.is_empty() {
        return Vec::new();
    }
    let methods = std::mem::take(&mut s.methods);
    let traits = std::mem::take(&mut s.implements);
    let span = decl.span.clone();
    let name_span = s.name_span.clone();
    let self_path = TypePath {
        segments: vec![TypePathSegment {
            name: s.name.clone(),
            generics: s
                .generics
                .iter()
                .map(|g| Type {
                    kind: TypeKind::Path(TypePath {
                        segments: vec![TypePathSegment {
                            name: g.name.clone(),
                            generics: Vec::new(),
                            span: g.span.clone(),
                        }],
                        span: g.span.clone(),
                    }),
                    span: g.span.clone(),
                })
                .collect(),
            span: name_span.clone(),
        }],
        span: name_span,
    };
    let generics = s.generics.clone();
    let fresh = || {
        let id = crate::span::next_copy_id();
        let mut p = self_path.clone();
        shift_path(&mut p, id);
        let gs: Vec<GenericParam> = generics
            .iter()
            .map(|g| {
                let mut g = g.clone();
                g.span = g.span.in_copy(id);
                for b in &mut g.bounds {
                    shift_path(b, id);
                }
                g
            })
            .collect();
        (p, gs, span.in_copy(id))
    };
    let mut out = Vec::new();
    {
        let (path, gs, sp) = fresh();
        out.push(Decl {
            kind: DeclKind::Impl(Impl {
                generics: gs,
                trait_or_type: path,
                for_type: None,
                items: methods,
                from_class: true,
                span: sp.clone(),
            }),
            span: sp,
        });
    }
    for t in traits {
        let (path, gs, sp) = fresh();
        out.push(Decl {
            kind: DeclKind::Impl(Impl {
                generics: gs,
                trait_or_type: t,
                for_type: Some(path),
                items: Vec::new(),
                from_class: true,
                span: sp.clone(),
            }),
            span: sp,
        });
    }
    out
}

fn shift_path(p: &mut TypePath, d: usize) {
    p.span = p.span.in_copy(d);
    for seg in &mut p.segments {
        seg.span = seg.span.in_copy(d);
        for g in &mut seg.generics {
            shift_type(g, d);
        }
    }
}

fn shift_type(t: &mut Type, d: usize) {
    t.span = t.span.in_copy(d);
    match &mut t.kind {
        TypeKind::Path(p) | TypeKind::Dyn(p) => shift_path(p, d),
        TypeKind::Optional(inner) => shift_type(inner, d),
        TypeKind::Function { params, ret } => {
            for x in params {
                shift_type(x, d);
            }
            shift_type(ret, d);
        }
        TypeKind::Unit => {}
    }
}

fn wildcard(span: &Span) -> Pattern {
    Pattern {
        kind: PatternKind::Wildcard,
        span: span.clone(),
    }
}

fn block_expr(b: Block) -> Expr {
    Expr {
        span: b.span.clone(),
        kind: ExprKind::Block(b),
    }
}

fn empty_block(span: &Span) -> Block {
    Block {
        stmts: Vec::new(),
        trailing: None,
        span: span.clone(),
    }
}

/// Turn a block's tail value into a statement, so the block has type `Unit`.
fn statement_block(mut b: Block) -> Block {
    if let Some(t) = b.trailing.take() {
        let span = t.span.clone();
        b.stmts.push(Stmt {
            kind: StmtKind::Expr(*t),
            span,
        });
    }
    b
}

fn lower_expr(e: &mut Expr) {
    let span = e.span.clone();
    let kind = std::mem::replace(&mut e.kind, ExprKind::Bool(false));
    e.kind = match kind {
        ExprKind::Binary {
            op: op @ (BinaryOp::In | BinaryOp::NotIn),
            lhs,
            rhs,
        } => {
            let call = method(
                *rhs,
                "contains",
                vec![*lhs],
                &span.in_copy(crate::span::next_copy_id()),
            );
            if op == BinaryOp::In {
                call.kind
            } else {
                ExprKind::Unary {
                    op: crate::ast::UnaryOp::Not,
                    operand: Box::new(call),
                }
            }
        }
        ExprKind::Index { receiver, index } if matches!(index.kind, ExprKind::Range { .. }) => {
            let ExprKind::Range {
                start,
                end,
                inclusive,
            } = index.kind
            else {
                unreachable!()
            };
            let end = if inclusive {
                let s = end.span.in_copy(crate::span::next_copy_id());
                Expr {
                    kind: ExprKind::Binary {
                        op: BinaryOp::Add,
                        lhs: end,
                        rhs: Box::new(Expr {
                            kind: ExprKind::Int(1),
                            span: s.in_copy(crate::span::next_copy_id()),
                        }),
                    },
                    span: s,
                }
            } else {
                *end
            };
            method(
                *receiver,
                "slice",
                vec![*start, end],
                &span.in_copy(crate::span::next_copy_id()),
            )
            .kind
        }
        ExprKind::Comprehension {
            kind,
            key,
            value,
            pattern,
            iter,
            cond,
        } => comprehension(kind, key, *value, pattern, *iter, cond, &span),
        ExprKind::Cast { value, ty } => {
            let target = match &ty.kind {
                TypeKind::Path(p) if p.segments.len() == 1 => p.segments[0].name.to_lowercase(),
                _ => "unsupported".into(),
            };
            let name = match target.as_str() {
                "string" | "str" => "to_string".to_string(),
                other => format!("as_{other}"),
            };
            method(
                *value,
                &name,
                Vec::new(),
                &span.in_copy(crate::span::next_copy_id()),
            )
            .kind
        }
        ExprKind::Tuple(items) if items.is_empty() => ExprKind::Block(empty_block(&span)),
        ExprKind::Tuple(items) => ExprKind::Call {
            callee: Box::new(Expr {
                kind: ExprKind::Ident {
                    name: format!("Tuple{}", items.len()),
                    generics: Vec::new(),
                },
                span: span.in_copy(crate::span::next_copy_id()),
            }),
            args: items,
        },
        ExprKind::Match {
            scrutinee,
            mut arms,
        } => {
            for arm in &mut arms {
                tuple_patterns(&mut arm.pattern);
            }
            ExprKind::Match { scrutinee, arms }
        }
        ExprKind::For {
            mut pattern,
            mut iter,
            mut body,
        } => {
            if !matches!(iter.kind, ExprKind::Range { .. }) {
                let s = iter.span.clone();
                let receiver = std::mem::replace(
                    &mut *iter,
                    Expr {
                        kind: ExprKind::Bool(false),
                        span: s.clone(),
                    },
                );
                *iter = method(
                    receiver,
                    "__for_iter",
                    Vec::new(),
                    &s.in_copy(crate::span::next_copy_id()),
                );
            }
            if matches!(pattern.kind, PatternKind::Tuple { name: None, .. }) {
                let name = format!("__item{}", crate::span::next_copy_id());
                let item_span = pattern.span.in_copy(crate::span::next_copy_id());
                let whole = std::mem::replace(
                    &mut pattern,
                    Pattern {
                        kind: PatternKind::Ident(name.clone()),
                        span: item_span,
                    },
                );
                let init = Expr {
                    kind: ExprKind::Ident {
                        name,
                        generics: Vec::new(),
                    },
                    span: whole.span.in_copy(crate::span::next_copy_id()),
                };
                let mut stmts = Vec::new();
                destructure(&whole, init, false, &mut stmts);
                stmts.append(&mut body.stmts);
                body.stmts = stmts;
            }
            ExprKind::For {
                pattern,
                iter,
                body,
            }
        }
        ExprKind::IfLet {
            mut pattern,
            scrutinee,
            then_branch,
            else_branch,
        } => {
            tuple_patterns(&mut pattern);
            let key = Span {
                end: span.start + 2,
                ..span.clone()
            };
            let (then_branch, else_body) = match else_branch {
                Some(eb) => match *eb {
                    ElseBranch::Block(b) => (then_branch, block_expr(b)),
                    ElseBranch::If(x) => (then_branch, x),
                },
                None => (statement_block(then_branch), block_expr(empty_block(&key))),
            };
            let then_expr = block_expr(then_branch);
            ExprKind::Match {
                scrutinee,
                arms: vec![
                    MatchArm {
                        span: merge_spans(&pattern.span, &then_expr.span),
                        pattern,
                        guard: None,
                        body: then_expr,
                    },
                    MatchArm {
                        pattern: wildcard(&key),
                        guard: None,
                        span: else_body.span.clone(),
                        body: else_body,
                    },
                ],
            }
        }
        ExprKind::WhileLet {
            mut pattern,
            scrutinee,
            body,
        } => {
            tuple_patterns(&mut pattern);
            let key = Span {
                end: span.start + 5,
                ..span.clone()
            };
            let body = statement_block(body);
            let body_span = body.span.clone();
            let brk = Block {
                stmts: vec![Stmt {
                    kind: StmtKind::Break(None),
                    span: key.clone(),
                }],
                trailing: None,
                span: key.clone(),
            };
            let m = Expr {
                kind: ExprKind::Match {
                    scrutinee,
                    arms: vec![
                        MatchArm {
                            span: merge_spans(&pattern.span, &body_span),
                            pattern,
                            guard: None,
                            body: block_expr(body),
                        },
                        MatchArm {
                            pattern: wildcard(&key),
                            guard: None,
                            span: key.clone(),
                            body: block_expr(brk),
                        },
                    ],
                },
                span: span.clone(),
            };
            ExprKind::Loop(Block {
                stmts: vec![Stmt {
                    kind: StmtKind::Expr(m),
                    span: span.clone(),
                }],
                trailing: None,
                span,
            })
        }
        other => other,
    };
}

/// A bare `(p, q)` pattern as the struct pattern `Tuple2 { _0: p, _1: q }`.
fn tuple_patterns(p: &mut Pattern) {
    match &mut p.kind {
        PatternKind::Tuple { name, elements } => {
            for e in elements.iter_mut() {
                tuple_patterns(e);
            }
            if name.is_none() {
                let fields = std::mem::take(elements)
                    .into_iter()
                    .enumerate()
                    .map(|(i, e)| crate::ast::FieldPattern {
                        name: format!("_{i}"),
                        span: e.span.clone(),
                        pattern: Some(e),
                    })
                    .collect::<Vec<_>>();
                p.kind = PatternKind::Struct {
                    name: format!("Tuple{}", fields.len()),
                    fields,
                };
            }
        }
        PatternKind::Struct { fields, .. } => {
            for f in fields {
                if let Some(sub) = &mut f.pattern {
                    tuple_patterns(sub);
                }
            }
        }
        _ => {}
    }
}

/// Bind the parts of tuple pattern `p` from `init` as plain lets.
fn destructure(p: &Pattern, init: Expr, mutable: bool, out: &mut Vec<Stmt>) {
    let PatternKind::Tuple { elements, .. } = &p.kind else {
        return;
    };
    let tmp = format!("__tuple{}", crate::span::next_copy_id());
    let tmp_span = p.span.in_copy(crate::span::next_copy_id());
    out.push(Stmt {
        kind: StmtKind::Let {
            name: tmp.clone(),
            ty: None,
            init: Some(init),
            mutable: false,
        },
        span: tmp_span.clone(),
    });
    for (i, el) in elements.iter().enumerate() {
        let field = Expr {
            kind: ExprKind::Field {
                receiver: Box::new(Expr {
                    kind: ExprKind::Ident {
                        name: tmp.clone(),
                        generics: Vec::new(),
                    },
                    span: el.span.in_copy(crate::span::next_copy_id()),
                }),
                name: format!("_{i}"),
            },
            span: el.span.in_copy(crate::span::next_copy_id()),
        };
        match &el.kind {
            PatternKind::Ident(name) => out.push(Stmt {
                kind: StmtKind::Let {
                    name: name.clone(),
                    ty: None,
                    init: Some(field),
                    mutable,
                },
                span: el.span.clone(),
            }),
            PatternKind::Tuple { name: None, .. } => destructure(el, field, mutable, out),
            _ => {}
        }
    }
}

/// `let (a, b) = e` as `let t = e; let a = t._0; let b = t._1`.
fn lower_let_tuple(b: &mut Block) -> Result<(), RavenError> {
    if !b
        .stmts
        .iter()
        .any(|s| matches!(s.kind, StmtKind::LetTuple { .. }))
    {
        return Ok(());
    }
    let mut out = Vec::with_capacity(b.stmts.len() + 4);
    for stmt in std::mem::take(&mut b.stmts) {
        match stmt.kind {
            StmtKind::LetTuple {
                pattern,
                init,
                mutable,
            } => {
                if let Some(bad) = refutable_part(&pattern) {
                    return Err(RavenError::parse(
                        ParseError::Custom(
                            "a `let` tuple pattern takes names, `_`, or nested tuples; use `match` for anything else".into(),
                        ),
                        bad,
                    ));
                }
                destructure(&pattern, init, mutable, &mut out);
            }
            _ => out.push(stmt),
        }
    }
    b.stmts = out;
    Ok(())
}

fn refutable_part(p: &Pattern) -> Option<Span> {
    match &p.kind {
        PatternKind::Wildcard | PatternKind::Ident(_) => None,
        PatternKind::Tuple {
            name: None,
            elements,
        } => elements.iter().find_map(refutable_part),
        _ => Some(p.span.clone()),
    }
}

/// `let P = e else { .. }` as `let x = match e { P -> x, _ -> { .. } }` for
/// the one name `P` binds.
fn lower_let_else(b: &mut Block) -> Result<(), RavenError> {
    for stmt in &mut b.stmts {
        let StmtKind::LetElse { .. } = &stmt.kind else {
            continue;
        };
        let span = stmt.span.clone();
        let kind = std::mem::replace(&mut stmt.kind, StmtKind::Continue);
        let StmtKind::LetElse {
            pattern,
            init,
            else_block,
            mutable,
        } = kind
        else {
            unreachable!()
        };
        let key = Span {
            end: span.start + 3,
            ..span.clone()
        };
        let mut pattern = pattern;
        tuple_patterns(&mut pattern);
        let mut names = Vec::new();
        pattern_bindings(&pattern, &mut names);
        let else_arm = MatchArm {
            pattern: wildcard(&key),
            guard: None,
            span: else_block.span.clone(),
            body: block_expr(else_block),
        };
        stmt.kind = match names.len() {
            0 => {
                let unit = block_expr(empty_block(&pattern.span));
                StmtKind::Expr(Expr {
                    kind: ExprKind::Match {
                        scrutinee: Box::new(init),
                        arms: vec![
                            MatchArm {
                                span: pattern.span.clone(),
                                pattern,
                                guard: None,
                                body: unit,
                            },
                            else_arm,
                        ],
                    },
                    span,
                })
            }
            1 => {
                let name = names.remove(0);
                let value = Expr {
                    kind: ExprKind::Ident {
                        name: name.clone(),
                        generics: Vec::new(),
                    },
                    span: key.clone(),
                };
                StmtKind::Let {
                    name,
                    ty: None,
                    init: Some(Expr {
                        kind: ExprKind::Match {
                            scrutinee: Box::new(init),
                            arms: vec![
                                MatchArm {
                                    span: pattern.span.clone(),
                                    pattern,
                                    guard: None,
                                    body: value,
                                },
                                else_arm,
                            ],
                        },
                        span,
                    }),
                    mutable,
                }
            }
            _ => {
                return Err(RavenError::parse(
                    ParseError::Custom(
                        "a pattern `let ... else` can bind one name; match the value instead"
                            .into(),
                    ),
                    pattern.span.clone(),
                ))
            }
        };
    }
    Ok(())
}

/// Names a pattern binds, in order. A capitalized bare name is a variant.
fn pattern_bindings(p: &Pattern, out: &mut Vec<String>) {
    match &p.kind {
        PatternKind::Ident(n) => {
            if !n.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                out.push(n.clone());
            }
        }
        PatternKind::Tuple { elements, .. } => {
            for e in elements {
                pattern_bindings(e, out);
            }
        }
        PatternKind::Struct { fields, .. } => {
            for f in fields {
                match &f.pattern {
                    Some(sub) => pattern_bindings(sub, out),
                    None => out.push(f.name.clone()),
                }
            }
        }
        _ => {}
    }
}

/// `xs[N]` on a value is an index, though it parses like type arguments.
fn index_not_type_args(e: &mut Expr, values: &std::collections::HashSet<String>) {
    let ExprKind::Ident { name, generics } = &e.kind else {
        return;
    };
    if generics.len() != 1 || !values.contains(name) {
        return;
    }
    let TypeKind::Path(p) = &generics[0].kind else {
        return;
    };
    if p.segments.len() != 1 || !p.segments[0].generics.is_empty() {
        return;
    }
    let index_name = p.segments[0].name.clone();
    let index_span = generics[0].span.clone();
    let receiver = Expr {
        kind: ExprKind::Ident {
            name: name.clone(),
            generics: Vec::new(),
        },
        span: e.span.clone(),
    };
    let index = Expr {
        kind: ExprKind::Ident {
            name: index_name,
            generics: Vec::new(),
        },
        span: index_span.clone(),
    };
    e.span = merge_spans(&e.span, &index_span);
    e.kind = ExprKind::Index {
        receiver: Box::new(receiver),
        index: Box::new(index),
    };
}

/// `set()` and `dict()` are empty collection literals unless the program
/// declares a value or function of that name.
const BUILTINS: &[&str] = &[
    "set",
    "dict",
    "len",
    "sum",
    "min",
    "max",
    "sorted",
    "reversed",
    "enumerate",
    "zip",
    "str",
    "range",
];

fn is_builtin_name(n: &str) -> bool {
    BUILTINS.contains(&n)
}

fn method(receiver: Expr, name: &str, args: Vec<Expr>, span: &Span) -> Expr {
    Expr {
        kind: ExprKind::MethodCall {
            receiver: Box::new(receiver),
            name: name.to_string(),
            generics: Vec::new(),
            args,
        },
        span: span.clone(),
    }
}

/// Python-style builtins: `len(x)` is `x.len()`, `min(xs)` is
/// `xs.min().unwrap()`, `range(a, b)` is `a..b`, and so on, unless the file
/// defines or imports a function by that name.
fn builtin_call(e: &mut Expr, values: &std::collections::HashSet<String>) {
    let ExprKind::Call { callee, args } = &mut e.kind else {
        return;
    };
    let ExprKind::Ident { name, generics } = &callee.kind else {
        return;
    };
    if !generics.is_empty() || values.contains(name) || !is_builtin_name(name) {
        return;
    }
    if args
        .iter()
        .any(|a| matches!(a.kind, ExprKind::NamedArg { .. }))
    {
        return;
    }
    let name = name.clone();
    let span = e.span.clone();
    let inner = |s: &Span| s.in_copy(crate::span::next_copy_id());
    let mut args = std::mem::take(args);
    let one = |m: &str, args: &mut Vec<Expr>| method(args.remove(0), m, Vec::new(), &span);
    let new = match (name.as_str(), args.len()) {
        ("len", 1) => one("len", &mut args),
        ("sum", 1) => one("sum", &mut args),
        ("sorted", 1) => one("sorted", &mut args),
        ("reversed", 1) => one("reversed", &mut args),
        ("enumerate", 1) => one("enumerate", &mut args),
        ("str", 1) => one("to_string", &mut args),
        ("zip", 2) => {
            let a = args.remove(0);
            method(a, "zip", args, &span)
        }
        ("min" | "max", 1) => {
            let m = method(args.remove(0), &name, Vec::new(), &inner(&span));
            method(m, "unwrap", Vec::new(), &span)
        }
        ("min" | "max", 2) => Expr {
            kind: ExprKind::Call {
                callee: Box::new(Expr {
                    kind: ExprKind::Ident {
                        name: format!("__{name}2"),
                        generics: Vec::new(),
                    },
                    span: inner(&span),
                }),
                args,
            },
            span: span.clone(),
        },
        ("range", 1 | 2) => {
            let end = args.pop().expect("range end");
            let start = args.pop().unwrap_or(Expr {
                kind: ExprKind::Int(0),
                span: inner(&span),
            });
            Expr {
                kind: ExprKind::Range {
                    start: Box::new(start),
                    end: Box::new(end),
                    inclusive: false,
                },
                span: span.clone(),
            }
        }
        _ => {
            if let ExprKind::Call { args: a, .. } = &mut e.kind {
                *a = args;
            }
            return;
        }
    };
    *e = new;
}

fn empty_collection_call(e: &mut Expr, values: &std::collections::HashSet<String>) {
    let ExprKind::Call { callee, args } = &e.kind else {
        return;
    };
    let ExprKind::Ident { name, generics } = &callee.kind else {
        return;
    };
    if !args.is_empty() || !generics.is_empty() || values.contains(name) {
        return;
    }
    e.kind = match name.as_str() {
        "set" => ExprKind::SetLit(Vec::new()),
        "dict" => ExprKind::MapLit(Vec::new()),
        _ => return,
    };
}

/// `[v for p in xs if c]` as `{ let out = []; for p in xs { if c { out.push(v) } }; out }`,
/// and likewise for sets (`add`) and dicts (`set(k, v)`).
fn comprehension(
    kind: crate::ast::CompKind,
    key: Option<Box<Expr>>,
    value: Expr,
    pattern: Pattern,
    iter: Expr,
    cond: Option<Box<Expr>>,
    span: &Span,
) -> ExprKind {
    use crate::ast::CompKind;
    let at = || span.in_copy(crate::span::next_copy_id());
    let out = format!("__comp{}", crate::span::next_copy_id());
    let ident = |name: &str| Expr {
        kind: ExprKind::Ident {
            name: name.to_string(),
            generics: Vec::new(),
        },
        span: at(),
    };
    let (empty, add, args) = match kind {
        CompKind::List => (ExprKind::Array(Vec::new()), "push", vec![value]),
        CompKind::Set => (ExprKind::SetLit(Vec::new()), "add", vec![value]),
        CompKind::Dict => (
            ExprKind::MapLit(Vec::new()),
            "set",
            vec![*key.expect("a dict comprehension has a key"), value],
        ),
    };
    let push = Stmt {
        kind: StmtKind::Expr(method(ident(&out), add, args, &at())),
        span: at(),
    };
    let body_stmt = match cond {
        Some(c) => Stmt {
            kind: StmtKind::Expr(Expr {
                kind: ExprKind::If {
                    cond: c,
                    then_branch: Block {
                        stmts: vec![push],
                        trailing: None,
                        span: at(),
                    },
                    else_branch: None,
                },
                span: at(),
            }),
            span: at(),
        },
        None => push,
    };
    let mut for_loop = Expr {
        kind: ExprKind::For {
            pattern,
            iter: Box::new(iter),
            body: Block {
                stmts: vec![body_stmt],
                trailing: None,
                span: at(),
            },
        },
        span: at(),
    };
    lower_expr(&mut for_loop);
    ExprKind::Block(Block {
        stmts: vec![
            Stmt {
                kind: StmtKind::Let {
                    name: out.clone(),
                    ty: None,
                    init: Some(Expr {
                        kind: empty,
                        span: at(),
                    }),
                    mutable: false,
                },
                span: at(),
            },
            Stmt {
                kind: StmtKind::Expr(for_loop),
                span: at(),
            },
        ],
        trailing: Some(Box::new(ident(&out))),
        span: at(),
    })
}
