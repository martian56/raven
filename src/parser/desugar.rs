//! Lowers surface-only syntax to core AST after parsing: `class` bodies,
//! `if let`, `while let`, and `let ... else`. The formatter works on the
//! undesugared tree.

use crate::ast::visit;
use crate::ast::{
    Block, Decl, DeclKind, ElseBranch, Expr, ExprKind, File, GenericParam, Impl, MatchArm, Pattern,
    PatternKind, Stmt, StmtKind, Type, TypeKind, TypePath, TypePathSegment, VariantPayload,
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
                    if let Err(e) = lower_let_else(b) {
                        error = Some(e);
                    }
                }
            },
        );
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
    let mut copy = 0usize;
    let mut fresh = || {
        copy += 1;
        let d = copy << 50;
        let mut p = self_path.clone();
        shift_path(&mut p, d);
        let gs: Vec<GenericParam> = generics
            .iter()
            .map(|g| {
                let mut g = g.clone();
                g.span.start += d;
                g.span.end += d;
                for b in &mut g.bounds {
                    shift_path(b, d);
                }
                g
            })
            .collect();
        let mut sp = span.clone();
        sp.start += d;
        sp.end += d;
        (p, gs, sp)
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
    p.span.start += d;
    p.span.end += d;
    for seg in &mut p.segments {
        seg.span.start += d;
        seg.span.end += d;
        for g in &mut seg.generics {
            shift_type(g, d);
        }
    }
}

fn shift_type(t: &mut Type, d: usize) {
    t.span.start += d;
    t.span.end += d;
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
        ExprKind::IfLet {
            pattern,
            scrutinee,
            then_branch,
            else_branch,
        } => {
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
            pattern,
            scrutinee,
            body,
        } => {
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
