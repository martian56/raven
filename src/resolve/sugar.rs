//! Expansion of `class` bodies, constructor calls, named and default
//! arguments, and field defaults, run on the merged item list.

use std::collections::HashMap;

use crate::ast::respan::Respan;
use crate::ast::visit;
use crate::ast::{
    Block, Decl, DeclKind, Expr, ExprKind, FieldInit, Function, Param, Stmt, StmtKind, StructField,
};
use crate::error::{RavenError, ResolveError};

pub fn expand(items: &mut Vec<Decl>) -> Result<(), RavenError> {
    drop_shadowed_bundled_methods(items);
    distribute_class_methods(items);
    expand_calls(items, true)
}

/// Expand constructor calls and named arguments. With `strict` unset (a
/// single module before merging), calls it cannot resolve are left for the
/// whole-program pass.
pub fn expand_calls(items: &mut [Decl], strict: bool) -> Result<(), RavenError> {
    let mut structs = collect_structs(items);
    for d in items.iter() {
        if let DeclKind::Import(im) = &d.kind {
            for sel in &im.selectors {
                if let Some(alias) = &sel.alias {
                    if let Some(fields) = structs
                        .get(&sel.name)
                        .or_else(|| suffix_get(&structs, &sel.name))
                        .cloned()
                    {
                        structs.entry(alias.clone()).or_insert(fields);
                    }
                }
            }
        }
    }
    let funcs = collect_functions(items);
    let mut counter: usize = if strict { 1 << 20 } else { 1 << 19 } + (items.len() << 8);
    let mut error: Option<RavenError> = None;
    for decl in items.iter_mut() {
        visit::exprs_in_decl(decl, &mut |e: &mut Expr| {
            if error.is_some() {
                return;
            }
            if let Err(err) = rewrite(e, &structs, &funcs, &mut counter, strict) {
                error = Some(err);
            }
        });
    }
    match error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// A program's own inherent method replaces a bundled stdlib method of the
/// same name on the same type.
fn drop_shadowed_bundled_methods(items: &mut [Decl]) {
    let bundled = |d: &Decl| d.span.file.to_string_lossy().starts_with("<bundled>");
    let mut own: std::collections::HashSet<(String, String)> = Default::default();
    for d in items.iter() {
        if let DeclKind::Impl(imp) = &d.kind {
            if imp.for_type.is_none() && !bundled(d) {
                let ty = last_name(Some(&imp.trait_or_type.segments));
                for f in &imp.items {
                    own.insert((ty.clone(), f.name.clone()));
                }
            }
        }
    }
    if own.is_empty() {
        return;
    }
    for d in items.iter_mut() {
        if !bundled(d) {
            continue;
        }
        if let DeclKind::Impl(imp) = &mut d.kind {
            if imp.for_type.is_none() {
                let ty = last_name(Some(&imp.trait_or_type.segments));
                imp.items
                    .retain(|f| !own.contains(&(ty.clone(), f.name.clone())));
            }
        }
    }
}

/// Move each method of a class's inherent impl into the impl of the listed
/// trait that declares it.
pub(crate) fn distribute_class_methods(items: &mut [Decl]) {
    let mut trait_methods: HashMap<String, Vec<String>> = HashMap::new();
    for d in items.iter() {
        if let DeclKind::Trait(t) = &d.kind {
            trait_methods.insert(
                t.name.clone(),
                t.members.iter().map(|m| m.name.clone()).collect(),
            );
        }
    }
    let mut owner: HashMap<(String, String), usize> = HashMap::new();
    for (i, d) in items.iter().enumerate() {
        if let DeclKind::Impl(imp) = &d.kind {
            if !imp.from_class || imp.for_type.is_none() {
                continue;
            }
            let class = last_name(imp.for_type.as_ref().map(|p| &p.segments));
            let tr = last_name(Some(&imp.trait_or_type.segments));
            if let Some(names) = trait_methods
                .get(&tr)
                .or_else(|| suffix_get(&trait_methods, &tr))
            {
                for n in names {
                    owner.insert((class.clone(), n.clone()), i);
                }
            }
        }
    }
    if owner.is_empty() {
        return;
    }
    let mut moves: Vec<(usize, Function)> = Vec::new();
    for d in items.iter_mut() {
        if let DeclKind::Impl(imp) = &mut d.kind {
            if !imp.from_class || imp.for_type.is_some() {
                continue;
            }
            let class = last_name(Some(&imp.trait_or_type.segments));
            let mut kept = Vec::new();
            for f in imp.items.drain(..) {
                match owner.get(&(class.clone(), f.name.clone())) {
                    Some(&target) => moves.push((target, f)),
                    None => kept.push(f),
                }
            }
            imp.items = kept;
        }
    }
    for (target, f) in moves {
        if let DeclKind::Impl(imp) = &mut items[target].kind {
            imp.items.push(f);
        }
    }
}

fn last_name(segs: Option<&Vec<crate::ast::TypePathSegment>>) -> String {
    segs.and_then(|s| s.last())
        .map(|s| s.name.clone())
        .unwrap_or_default()
}

fn suffix_get<'a, V>(map: &'a HashMap<String, V>, name: &str) -> Option<&'a V> {
    let mut found = map
        .iter()
        .filter(|(k, _)| k.rsplit('.').next() == Some(name));
    match (found.next(), found.next()) {
        (Some((_, v)), None) => Some(v),
        _ => None,
    }
}

fn collect_structs(items: &[Decl]) -> HashMap<String, Vec<StructField>> {
    let mut out = HashMap::new();
    for d in items {
        if let DeclKind::Struct(s) = &d.kind {
            out.insert(s.name.clone(), s.fields.clone());
        }
    }
    out
}

fn collect_functions(items: &[Decl]) -> HashMap<String, Vec<Param>> {
    let mut out = HashMap::new();
    for d in items {
        if let DeclKind::Function(f) = &d.kind {
            if f.params.iter().any(|p| p.default.is_some()) {
                out.insert(f.name.clone(), f.params.clone());
            }
        }
    }
    out
}

fn fresh(counter: &mut usize, e: &Expr) -> Expr {
    *counter += 1;
    let mut copy = e.clone();
    let subst = HashMap::new();
    let mut tmp = Function {
        name: String::new(),
        generics: Vec::new(),
        params: Vec::new(),
        ret: None,
        body: crate::ast::FunctionBody::Expr(copy.clone()),
        span: e.span.clone(),
    };
    Respan::new(*counter << 40, &subst).function(&mut tmp);
    if let crate::ast::FunctionBody::Expr(x) = tmp.body {
        copy = x;
    }
    copy
}

fn shifted(counter: &mut usize, span: &crate::span::Span) -> crate::span::Span {
    *counter += 1;
    let mut s = span.clone();
    s.start += *counter << 40;
    s.end += *counter << 40;
    s
}

fn bad(msg: String, span: &crate::span::Span) -> RavenError {
    RavenError::resolve(ResolveError::Other(msg), span.clone())
}

fn rewrite(
    e: &mut Expr,
    structs: &HashMap<String, Vec<StructField>>,
    funcs: &HashMap<String, Vec<Param>>,
    counter: &mut usize,
    strict: bool,
) -> Result<(), RavenError> {
    if !strict {
        if let ExprKind::Call { callee, .. } = &e.kind {
            if let ExprKind::Ident { name, .. } = &callee.kind {
                if !structs.contains_key(name) && !funcs.contains_key(name) {
                    return Ok(());
                }
            }
        }
        if let ExprKind::MethodCall { .. } = &e.kind {
            return Ok(());
        }
    }
    match &mut e.kind {
        ExprKind::Call { callee, args } => {
            let ExprKind::Ident { name, generics } = &callee.kind else {
                return check_no_named(args);
            };
            let fields = structs
                .get(name)
                .or_else(|| suffix_get(structs, name))
                .filter(|_| name.chars().next().is_some_and(|c| c.is_ascii_uppercase()));
            if let Some(fields) = fields {
                let lit = construct(
                    name.clone(),
                    generics.clone(),
                    args,
                    fields,
                    &e.span,
                    counter,
                )?;
                *e = lit;
                return Ok(());
            }
            if let Some(params) = funcs.get(name).or_else(|| suffix_get(funcs, name)) {
                let ordered = order_args(name, args, params, &e.span, counter)?;
                *args = ordered;
                return Ok(());
            }
            check_no_named(args)
        }
        ExprKind::MethodCall { args, .. } => check_no_named(args),
        ExprKind::StructLit { name, fields, .. } => {
            let Some(decl) = structs.get(name).or_else(|| suffix_get(structs, name)) else {
                return Ok(());
            };
            for f in decl {
                if let Some(d) = &f.default {
                    if !fields.iter().any(|x| x.name == f.name) {
                        fields.push(FieldInit {
                            name: f.name.clone(),
                            value: fresh(counter, d),
                            span: e.span.clone(),
                        });
                    }
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn check_no_named(args: &[Expr]) -> Result<(), RavenError> {
    for a in args {
        if let ExprKind::NamedArg { name, .. } = &a.kind {
            let msg = if name == ".." {
                "`..base` only works when constructing a class".to_string()
            } else {
                format!("named argument `{name}` only works on a function or class constructor")
            };
            return Err(bad(msg, &a.span));
        }
    }
    Ok(())
}

/// `Point(1, y: 2, ..base)` as a struct literal.
fn construct(
    name: String,
    generics: Vec<crate::ast::Type>,
    args: &mut Vec<Expr>,
    fields: &[StructField],
    span: &crate::span::Span,
    counter: &mut usize,
) -> Result<Expr, RavenError> {
    let mut inits: Vec<FieldInit> = Vec::new();
    let mut base: Option<Expr> = None;
    let mut positional = 0usize;
    for a in args.drain(..) {
        match a.kind {
            ExprKind::NamedArg { name: n, value } if n == ".." => base = Some(*value),
            ExprKind::NamedArg { name: n, value } => {
                if !fields.iter().any(|f| f.name == n) {
                    return Err(bad(format!("`{name}` has no field `{n}`"), &a.span));
                }
                if inits.iter().any(|i| i.name == n) {
                    return Err(bad(format!("field `{n}` is given twice"), &a.span));
                }
                inits.push(FieldInit {
                    name: n,
                    value: *value,
                    span: a.span,
                });
            }
            _ => {
                let Some(f) = fields.get(positional) else {
                    return Err(bad(
                        format!(
                            "`{name}` has {} field(s), but more arguments were given",
                            fields.len()
                        ),
                        &a.span,
                    ));
                };
                positional += 1;
                inits.push(FieldInit {
                    name: f.name.clone(),
                    span: a.span.clone(),
                    value: a,
                });
            }
        }
    }
    let base_var = base.as_ref().map(|_| format!("__base{}", *counter));
    for f in fields {
        if inits.iter().any(|i| i.name == f.name) {
            continue;
        }
        if let Some(var) = &base_var {
            let use_span = shifted(counter, span);
            let field_span = shifted(counter, span);
            inits.push(FieldInit {
                name: f.name.clone(),
                value: Expr {
                    kind: ExprKind::Field {
                        receiver: Box::new(Expr {
                            kind: ExprKind::Ident {
                                name: var.clone(),
                                generics: Vec::new(),
                            },
                            span: use_span,
                        }),
                        name: f.name.clone(),
                    },
                    span: field_span.clone(),
                },
                span: field_span,
            });
        } else if let Some(d) = &f.default {
            inits.push(FieldInit {
                name: f.name.clone(),
                value: fresh(counter, d),
                span: span.clone(),
            });
        }
    }
    let lit = Expr {
        kind: ExprKind::StructLit {
            name,
            generics,
            fields: inits,
        },
        span: span.clone(),
    };
    let (Some(var), Some(base)) = (base_var, base) else {
        return Ok(lit);
    };
    let let_span = shifted(counter, span);
    let block_span = shifted(counter, span);
    let lit_span = shifted(counter, span);
    let mut lit = lit;
    lit.span = lit_span;
    Ok(Expr {
        kind: ExprKind::Block(Block {
            stmts: vec![Stmt {
                kind: StmtKind::Let {
                    name: var,
                    ty: None,
                    init: Some(base),
                    mutable: false,
                },
                span: let_span,
            }],
            trailing: Some(Box::new(lit)),
            span: block_span.clone(),
        }),
        span: block_span,
    })
}

/// Put named arguments in parameter order and fill defaults.
fn order_args(
    fname: &str,
    args: &mut Vec<Expr>,
    params: &[Param],
    span: &crate::span::Span,
    counter: &mut usize,
) -> Result<Vec<Expr>, RavenError> {
    let mut slots: Vec<Option<Expr>> = vec![None; params.len()];
    let mut positional = 0usize;
    for a in args.drain(..) {
        match a.kind {
            ExprKind::NamedArg { name, value } => {
                let Some(i) = params.iter().position(|p| p.name == name) else {
                    return Err(bad(format!("`{fname}` has no parameter `{name}`"), &a.span));
                };
                if slots[i].is_some() {
                    return Err(bad(format!("argument `{name}` is given twice"), &a.span));
                }
                slots[i] = Some(*value);
            }
            _ => {
                if positional >= params.len() {
                    return Err(bad(
                        format!(
                            "`{fname}` takes {} argument(s), but more were given",
                            params.len()
                        ),
                        &a.span,
                    ));
                }
                slots[positional] = Some(a);
                positional += 1;
            }
        }
    }
    let mut out = Vec::with_capacity(params.len());
    for (slot, p) in slots.into_iter().zip(params) {
        match (slot, &p.default) {
            (Some(v), _) => out.push(v),
            (None, Some(d)) => out.push(fresh(counter, d)),
            (None, None) => {
                return Err(bad(
                    format!("missing argument `{}` in call to `{fname}`", p.name),
                    span,
                ))
            }
        }
    }
    Ok(out)
}
