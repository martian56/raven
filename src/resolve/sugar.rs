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
    let scope = Visible::build(items);
    let mut error: Option<RavenError> = None;
    for decl in items.iter_mut() {
        let file = decl.span.file.clone();
        // A module-level initializer (`let ORIGIN = Point(0, 0)`, or a
        // tuple, which desugars to a `TupleN(...)` call) is expanded too.
        let mut init = |e: &mut Expr| {
            let locals = init_locals(e);
            visit::expr(e, &mut |x: &mut Expr| {
                if error.is_some() {
                    return;
                }
                if let Err(err) = rewrite(x, &scope, &file, &locals, strict) {
                    error = Some(err);
                }
            });
        };
        match &mut decl.kind {
            DeclKind::Let(l) => {
                if let Some(e) = l.init.as_mut() {
                    init(e);
                }
            }
            DeclKind::Const(c) => init(&mut c.value),
            _ => {}
        }
        let mut each = |func: &mut Function| {
            let locals = local_names(func);
            visit::exprs_in_fn(func, &mut |e: &mut Expr| {
                if error.is_some() {
                    return;
                }
                if let Err(err) = rewrite(e, &scope, &file, &locals, strict) {
                    error = Some(err);
                }
            });
        };
        match &mut decl.kind {
            DeclKind::Function(f) => each(f),
            DeclKind::Impl(i) => i.items.iter_mut().for_each(&mut each),
            DeclKind::Trait(t) => t.members.iter_mut().for_each(&mut each),
            _ => {}
        }
    }
    match error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

type FileKey = std::sync::Arc<std::path::PathBuf>;

/// Which classes and default-argument functions each file can name.
struct Visible {
    /// Declarations by name, with the file each is declared in.
    structs: HashMap<String, Vec<(FileKey, Vec<StructField>)>>,
    funcs: HashMap<String, Vec<(FileKey, Vec<Param>)>>,
    /// Per enum, its named-field variants and their fields.
    variants: HashMap<String, Vec<(FileKey, HashMap<String, Vec<StructField>>)>>,
    /// Per file, local name to imported name from its selector imports.
    imports: HashMap<FileKey, HashMap<String, String>>,
    /// Every top-level name each file declares.
    declared: HashMap<FileKey, std::collections::HashSet<String>>,
}

impl Visible {
    fn build(items: &[Decl]) -> Self {
        let mut v = Visible {
            structs: HashMap::new(),
            funcs: HashMap::new(),
            variants: HashMap::new(),
            imports: HashMap::new(),
            declared: HashMap::new(),
        };
        for d in items {
            let name = match &d.kind {
                DeclKind::Struct(s) => Some(&s.name),
                DeclKind::Function(f) => Some(&f.name),
                DeclKind::Enum(e) => Some(&e.name),
                _ => None,
            };
            if let Some(n) = name {
                v.declared
                    .entry(d.span.file.clone())
                    .or_default()
                    .insert(n.clone());
            }
            match &d.kind {
                DeclKind::Struct(s) => v
                    .structs
                    .entry(s.name.clone())
                    .or_default()
                    .push((d.span.file.clone(), s.fields.clone())),
                DeclKind::Function(f) if f.params.iter().any(|p| p.default.is_some()) => v
                    .funcs
                    .entry(f.name.clone())
                    .or_default()
                    .push((d.span.file.clone(), f.params.clone())),
                DeclKind::Enum(e) => {
                    let mut named = HashMap::new();
                    for var in &e.variants {
                        let crate::ast::VariantPayload::Tuple(tys) = &var.payload else {
                            continue;
                        };
                        if var.field_names.len() != tys.len() {
                            continue;
                        }
                        let fields = var
                            .field_names
                            .iter()
                            .zip(tys)
                            .map(|(n, t)| StructField {
                                name: n.clone(),
                                ty: t.clone(),
                                default: None,
                                span: var.span.clone(),
                            })
                            .collect();
                        named.insert(var.name.clone(), fields);
                    }
                    if !named.is_empty() {
                        v.variants
                            .entry(e.name.clone())
                            .or_default()
                            .push((d.span.file.clone(), named));
                    }
                }
                DeclKind::Import(im) => {
                    let m = v.imports.entry(d.span.file.clone()).or_default();
                    for sel in &im.selectors {
                        let local = sel.alias.clone().unwrap_or_else(|| sel.name.clone());
                        m.insert(local, sel.name.clone());
                    }
                }
                _ => {}
            }
        }
        v
    }

    fn lookup<'a, T>(
        &self,
        table: &'a HashMap<String, Vec<(FileKey, T)>>,
        name: &str,
        file: &FileKey,
    ) -> Option<&'a T> {
        if let Some(entries) = table.get(name) {
            if name.contains('.') && entries.len() == 1 {
                return Some(&entries[0].1);
            }
            if let Some((_, t)) = entries.iter().find(|(f, _)| f == file) {
                return Some(t);
            }
        }
        let Some(imported) = self.imports.get(file).and_then(|m| m.get(name)) else {
            let shadowed = self.declared.get(file).is_some_and(|d| d.contains(name));
            return match table.get(name).map(|v| v.as_slice()) {
                Some([(_, t)]) if !shadowed => Some(t),
                _ => None,
            };
        };
        let mut found = table
            .iter()
            .filter(|(k, _)| *k == imported || k.rsplit('.').next() == Some(imported.as_str()))
            .flat_map(|(_, v)| v.iter());
        match (found.next(), found.next()) {
            (Some((_, t)), None) => Some(t),
            _ => None,
        }
    }
}

/// Every name a function binds as a value: parameters, `let`/`var`, lambda
/// parameters, and pattern bindings.
/// Names a module-level initializer binds inside itself (lambda parameters,
/// and the locals of block-bodied lambdas), which shadow a class name.
fn init_locals(e: &Expr) -> std::collections::HashSet<String> {
    let func = Function {
        name: String::new(),
        generics: Vec::new(),
        params: Vec::new(),
        ret: None,
        body: crate::ast::FunctionBody::Expr(e.clone()),
        span: e.span.clone(),
    };
    local_names(&func)
}

pub(crate) fn local_names(func: &Function) -> std::collections::HashSet<String> {
    let mut out: std::collections::HashSet<String> =
        func.params.iter().map(|p| p.name.clone()).collect();
    let mut copy = func.clone();
    let mut d = Decl {
        span: copy.span.clone(),
        kind: DeclKind::Function(std::mem::replace(
            &mut copy,
            Function {
                name: String::new(),
                generics: Vec::new(),
                params: Vec::new(),
                ret: None,
                body: crate::ast::FunctionBody::None,
                span: func.span.clone(),
            },
        )),
    };
    let names = std::cell::RefCell::new(Vec::new());
    visit::walk_decl(
        &mut d,
        &mut |e: &mut Expr| match &e.kind {
            ExprKind::Lambda { params, .. } => {
                names
                    .borrow_mut()
                    .extend(params.iter().map(|p| p.name.clone()));
            }
            ExprKind::For { pattern, .. } => pattern_names(pattern, &mut names.borrow_mut()),
            ExprKind::Match { arms, .. } => {
                for a in arms {
                    pattern_names(&a.pattern, &mut names.borrow_mut());
                }
            }
            _ => {}
        },
        &mut |b: &mut Block| {
            for st in &b.stmts {
                if let StmtKind::Let { name, .. } = &st.kind {
                    names.borrow_mut().push(name.clone());
                }
            }
        },
    );
    out.extend(names.into_inner());
    out
}

fn pattern_names(p: &crate::ast::Pattern, out: &mut Vec<String>) {
    use crate::ast::PatternKind;
    match &p.kind {
        PatternKind::Ident(n) => out.push(n.clone()),
        PatternKind::Tuple { elements, .. } => {
            for e in elements {
                pattern_names(e, out);
            }
        }
        PatternKind::Struct { fields, .. } => {
            for f in fields {
                match &f.pattern {
                    Some(sub) => pattern_names(sub, out),
                    None => out.push(f.name.clone()),
                }
            }
        }
        _ => {}
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

fn fresh(e: &Expr) -> Expr {
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
    Respan::new(crate::span::next_copy_id(), &subst).function(&mut tmp);
    if let crate::ast::FunctionBody::Expr(x) = tmp.body {
        copy = x;
    }
    copy
}

fn shifted(span: &crate::span::Span) -> crate::span::Span {
    let mut s = span.clone();
    s = s.in_copy(crate::span::next_copy_id());
    s
}

fn bad(msg: String, span: &crate::span::Span) -> RavenError {
    RavenError::resolve(ResolveError::Other(msg), span.clone())
}

fn rewrite(
    e: &mut Expr,
    scope: &Visible,
    file: &FileKey,
    locals: &std::collections::HashSet<String>,
    strict: bool,
) -> Result<(), RavenError> {
    match &mut e.kind {
        ExprKind::Call { callee, args } => {
            let ExprKind::Ident { name, generics } = &callee.kind else {
                return if strict { check_no_named(args) } else { Ok(()) };
            };
            if locals.contains(name) {
                return if strict { check_no_named(args) } else { Ok(()) };
            }
            let fields = scope.lookup(&scope.structs, name, file).filter(|_| {
                name.rsplit('.')
                    .next()
                    .and_then(|n| n.trim_start_matches('_').chars().next())
                    .is_some_and(|c| c.is_ascii_uppercase())
            });
            if let Some(fields) = fields {
                let lit = construct(name.clone(), generics.clone(), args, fields, &e.span)?;
                *e = lit;
                return Ok(());
            }
            if let Some(params) = scope.lookup(&scope.funcs, name, file) {
                let ordered = order_args(name, args, params, &e.span)?;
                *args = ordered;
                return Ok(());
            }
            if let Some((owner, variant)) = name.rsplit_once('.') {
                let fields = scope
                    .lookup(&scope.variants, owner, file)
                    .and_then(|m| m.get(variant));
                if let Some(fields) = fields {
                    let params = variant_params(fields);
                    *args = order_args(name, args, &params, &e.span)?;
                    return Ok(());
                }
            }
            if strict {
                check_no_named(args)
            } else {
                Ok(())
            }
        }
        ExprKind::MethodCall {
            receiver,
            name,
            args,
            ..
        } => {
            if let ExprKind::Ident { name: owner, .. } = &receiver.kind {
                let key = format!("{owner}.{name}");
                if !locals.contains(owner) {
                    let fields = scope
                        .lookup(&scope.variants, owner, file)
                        .and_then(|m| m.get(name.as_str()));
                    if let Some(fields) = fields {
                        let params = variant_params(fields);
                        *args = order_args(&key, args, &params, &e.span)?;
                        return Ok(());
                    }
                }
            }
            if strict {
                check_no_named(args)
            } else {
                Ok(())
            }
        }
        ExprKind::StructLit { name, fields, .. } => {
            let Some(decl) = scope.lookup(&scope.structs, name, file) else {
                return Ok(());
            };
            for f in decl {
                if let Some(d) = &f.default {
                    if !fields.iter().any(|x| x.name == f.name) {
                        fields.push(FieldInit {
                            name: f.name.clone(),
                            value: fresh(d),
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

fn variant_params(fields: &[StructField]) -> Vec<Param> {
    fields
        .iter()
        .map(|f| Param {
            name: f.name.clone(),
            ty: f.ty.clone(),
            default: f.default.clone(),
            span: f.span.clone(),
        })
        .collect()
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
    let base_var = base
        .as_ref()
        .map(|_| format!("__base{}", crate::span::next_copy_id()));
    for f in fields {
        if inits.iter().any(|i| i.name == f.name) {
            continue;
        }
        if let Some(var) = &base_var {
            let use_span = shifted(span);
            let field_span = shifted(span);
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
                value: fresh(d),
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
    let let_span = shifted(span);
    let block_span = shifted(span);
    let lit_span = shifted(span);
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
                if slots.get(positional).is_some_and(|s| s.is_some()) {
                    return Err(bad(
                        format!(
                            "a positional argument follows a named one in `{fname}`; put positional arguments first"
                        ),
                        &a.span,
                    ));
                }
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
            (None, Some(d)) => out.push(fresh(d)),
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
