//! Body walk: the second resolver pass.
//!
//! Walks every declaration body and inner expression in the file,
//! recording use site bindings in a [`ResolutionMap`]. The module
//! scope and import bindings must already be populated by
//! [`super::items::collect_items`] and
//! [`super::imports::resolve_imports`].
//!
//! The walker is structured as recursive `walk_*` functions, one per
//! AST node category. Each function owns its local scope frames and is
//! responsible for popping anything it pushes.

use crate::ast::{
    Block, Decl, DeclKind, Enum, ExprKind, File, FunctionBody, Impl, LambdaBody, Pattern,
    PatternKind, Stmt, StmtKind, Trait, TypeKind, TypePath, VariantPayload,
};
use crate::ast::{Expr, Struct, Type};
use crate::error::{RavenError, ResolveError};
use crate::span::Span;

use super::bindings::{Binding, DeclId, ResolutionMap};
use super::scope::{ScopeKind, ScopeStack};

/// Insert a parameter binding, rejecting a name that already appears in the
/// same parameter list (which would otherwise silently shadow the earlier one,
/// so reads selected the last). A repeated `_` is allowed: it never binds a
/// usable name.
fn insert_param(scope: &mut ScopeStack, name: &str, span: Span) -> Result<(), RavenError> {
    if name == "_" {
        scope.insert_shadowing(name, Binding::Param(span.clone()), span);
        Ok(())
    } else {
        scope.insert(name, Binding::Param(span.clone()), span)
    }
}

/// Walk every body in `file`, recording bindings in `map`.
pub fn walk_file(
    file: &File,
    scope: &mut ScopeStack,
    map: &mut ResolutionMap,
) -> Result<(), RavenError> {
    for (idx, decl) in file.items.iter().enumerate() {
        walk_decl(decl, DeclId(idx), scope, map)?;
    }
    Ok(())
}

fn walk_decl(
    decl: &Decl,
    id: DeclId,
    scope: &mut ScopeStack,
    map: &mut ResolutionMap,
) -> Result<(), RavenError> {
    match &decl.kind {
        DeclKind::Function(f) => {
            scope.push(ScopeKind::Function);
            push_generics(scope, &f.generics, &decl.span)?;
            for p in &f.params {
                insert_param(scope, &p.name, p.span.clone())?;
                walk_type(&p.ty, scope, map)?;
            }
            if let Some(r) = &f.ret {
                walk_type(r, scope, map)?;
            }
            walk_function_body(&f.body, scope, map)?;
            scope.pop();
        }
        DeclKind::Struct(s) => walk_struct(s, scope, map)?,
        DeclKind::Trait(t) => walk_trait(t, scope, map)?,
        DeclKind::Enum(e) => walk_enum(e, scope, map)?,
        DeclKind::Impl(i) => walk_impl(i, id, scope, map)?,
        DeclKind::Extern(ext) => {
            for item in &ext.items {
                for p in &item.params {
                    walk_type(&p.ty, scope, map)?;
                }
                if let Some(r) = &item.ret {
                    walk_type(r, scope, map)?;
                }
            }
        }
        DeclKind::Const(c) => {
            if let Some(t) = &c.ty {
                walk_type(t, scope, map)?;
            }
            walk_expr(&c.value, scope, map)?;
        }
        DeclKind::Let(l) => {
            if let Some(t) = &l.ty {
                walk_type(t, scope, map)?;
            }
            if let Some(e) = &l.init {
                walk_expr(e, scope, map)?;
            }
        }
        DeclKind::Import(_) => {
            // Already resolved by the imports pass.
        }
        DeclKind::Macro(_) => {
            // Macros are expanded before the compiler parses; only the
            // formatter produces this node, so resolution has nothing to do.
        }
    }
    Ok(())
}

fn walk_struct(
    s: &Struct,
    scope: &mut ScopeStack,
    map: &mut ResolutionMap,
) -> Result<(), RavenError> {
    scope.push(ScopeKind::Function);
    push_generics(scope, &s.generics, &s.span)?;
    for f in &s.fields {
        walk_type(&f.ty, scope, map)?;
    }
    scope.pop();
    Ok(())
}

fn walk_trait(
    t: &Trait,
    scope: &mut ScopeStack,
    map: &mut ResolutionMap,
) -> Result<(), RavenError> {
    // A trait body is an implicit `Self`-bearing context: methods may
    // take `self`, annotate parameters as `Self`, and reference `Self`
    // in their signatures, exactly like an `impl` block. Push an `Impl`
    // frame and bind `Self` so those uses resolve rather than reporting
    // `SelfOutsideImpl`.
    scope.push(ScopeKind::Impl);
    let _ = scope.insert("Self", Binding::SelfType, t.span.clone());
    push_generics(scope, &t.generics, &t.span)?;
    for member in &t.members {
        scope.push(ScopeKind::Impl);
        push_generics(scope, &member.generics, &member.span)?;
        for p in &member.params {
            if p.name == "self" {
                scope.insert_shadowing("self", Binding::SelfValue, p.span.clone());
            } else {
                insert_param(scope, &p.name, p.span.clone())?;
                walk_type(&p.ty, scope, map)?;
            }
        }
        if let Some(r) = &member.ret {
            walk_type(r, scope, map)?;
        }
        walk_function_body(&member.body, scope, map)?;
        scope.pop();
    }
    scope.pop();
    Ok(())
}

fn walk_enum(e: &Enum, scope: &mut ScopeStack, map: &mut ResolutionMap) -> Result<(), RavenError> {
    scope.push(ScopeKind::Function);
    push_generics(scope, &e.generics, &e.span)?;
    for v in &e.variants {
        match &v.payload {
            VariantPayload::Unit => {}
            VariantPayload::Tuple(tys) => {
                for t in tys {
                    walk_type(t, scope, map)?;
                }
            }
            VariantPayload::Struct(fields) => {
                for f in fields {
                    walk_type(&f.ty, scope, map)?;
                }
            }
        }
    }
    scope.pop();
    Ok(())
}

fn walk_impl(
    i: &Impl,
    _id: DeclId,
    scope: &mut ScopeStack,
    map: &mut ResolutionMap,
) -> Result<(), RavenError> {
    scope.push(ScopeKind::Impl);
    push_generics(scope, &i.generics, &i.span)?;
    // Bind Self to the implementing type. The "implementing type" is
    // `for_type` if present (trait impl), else `trait_or_type` (inherent
    // impl).
    let _ = scope.insert("Self", Binding::SelfType, i.span.clone());
    // Resolve the type path(s) in the head.
    walk_type_path(&i.trait_or_type, scope, map)?;
    if let Some(for_ty) = &i.for_type {
        walk_type_path(for_ty, scope, map)?;
    }
    for item in &i.items {
        scope.push(ScopeKind::Function);
        push_generics(scope, &item.generics, &item.span)?;
        for p in &item.params {
            if p.name == "self" {
                scope.insert_shadowing("self", Binding::SelfValue, p.span.clone());
            } else {
                insert_param(scope, &p.name, p.span.clone())?;
                walk_type(&p.ty, scope, map)?;
            }
        }
        if let Some(r) = &item.ret {
            walk_type(r, scope, map)?;
        }
        walk_function_body(&item.body, scope, map)?;
        scope.pop();
    }
    scope.pop();
    Ok(())
}

fn push_generics(
    scope: &mut ScopeStack,
    generics: &[crate::ast::GenericParam],
    owner: &Span,
) -> Result<(), RavenError> {
    for g in generics {
        scope.insert(
            &g.name,
            Binding::GenericParam {
                owner: owner.clone(),
                name: g.name.clone(),
            },
            g.span.clone(),
        )?;
        // Trait bounds are themselves type paths and need their leading
        // name resolved against the current scope.
        for bound in &g.bounds {
            walk_type_path(bound, scope, &mut ResolutionMap::new())?;
        }
    }
    Ok(())
}

fn walk_function_body(
    body: &FunctionBody,
    scope: &mut ScopeStack,
    map: &mut ResolutionMap,
) -> Result<(), RavenError> {
    match body {
        FunctionBody::Block(b) => walk_block(b, scope, map),
        FunctionBody::Expr(e) => walk_expr(e, scope, map),
        FunctionBody::None => Ok(()),
    }
}

fn walk_block(
    block: &Block,
    scope: &mut ScopeStack,
    map: &mut ResolutionMap,
) -> Result<(), RavenError> {
    scope.push(ScopeKind::Block);
    for stmt in &block.stmts {
        walk_stmt(stmt, scope, map)?;
    }
    if let Some(t) = &block.trailing {
        walk_expr(t, scope, map)?;
    }
    scope.pop();
    Ok(())
}

fn walk_stmt(
    stmt: &Stmt,
    scope: &mut ScopeStack,
    map: &mut ResolutionMap,
) -> Result<(), RavenError> {
    match &stmt.kind {
        StmtKind::Let {
            name,
            ty,
            init,
            mutable: _,
        } => {
            if let Some(t) = ty {
                walk_type(t, scope, map)?;
            }
            if let Some(e) = init {
                walk_expr(e, scope, map)?;
            }
            scope.insert_shadowing(name, Binding::Local(stmt.span.clone()), stmt.span.clone());
        }
        StmtKind::Return(e) => {
            if let Some(e) = e {
                walk_expr(e, scope, map)?;
            }
        }
        StmtKind::Break(e) => {
            if let Some(e) = e {
                walk_expr(e, scope, map)?;
            }
        }
        StmtKind::Continue => {}
        StmtKind::Defer(e) | StmtKind::Spawn(e) => walk_expr(e, scope, map)?,
        StmtKind::Assign { target, value, .. } => {
            walk_expr(target, scope, map)?;
            walk_expr(value, scope, map)?;
        }
        StmtKind::Expr(e) => walk_expr(e, scope, map)?,
    }
    Ok(())
}

fn walk_expr(
    expr: &Expr,
    scope: &mut ScopeStack,
    map: &mut ResolutionMap,
) -> Result<(), RavenError> {
    match &expr.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::BlockStr(_)
        | ExprKind::Char(_)
        // A macro call only appears in formatter-parsed source (the compile
        // pipeline expands macros first), so there are no names to resolve.
        | ExprKind::MacroCall(_)
        | ExprKind::CStr(_) => {}
        ExprKind::SelfLower => {
            if !scope.in_impl() {
                return Err(RavenError::resolve(
                    ResolveError::SelfOutsideImpl,
                    expr.span.clone(),
                ));
            }
            // `self` is bound (as `SelfValue`) only when the enclosing method
            // declares it as a parameter. Without that, the back end has no
            // `self` value to produce, so reject the use here with a clear
            // message rather than letting it surface as a confusing codegen
            // error (a field access on a Unit `self`).
            if !matches!(
                scope.lookup("self").map(|e| &e.binding),
                Some(Binding::SelfValue)
            ) {
                return Err(RavenError::resolve(
                    ResolveError::SelfNotMethodParam,
                    expr.span.clone(),
                )
                .with_hint("add `self` as the method's first parameter: `fun name(self, ...)`"));
            }
            map.bind_use(&expr.span, Binding::SelfValue);
        }
        ExprKind::SelfUpper => {
            if !scope.in_impl() {
                return Err(RavenError::resolve(
                    ResolveError::SelfOutsideImpl,
                    expr.span.clone(),
                ));
            }
            map.bind_use(&expr.span, Binding::SelfType);
        }
        ExprKind::Ident { name, generics } => {
            // An explicit binding in scope always wins. This lets a user
            // shadow a builtin name by importing it: `import std/io { print }`
            // binds `print` to the stdlib function, which then takes
            // precedence over the built in `print` free function.
            //
            // A free identifier a macro template introduced resolves at the
            // macro's definition site (the module scope), not the call site,
            // so a caller's local of the same name cannot capture it.
            let resolved = if scope.is_def_site(&expr.span) {
                scope.lookup_module(name)
            } else {
                scope.lookup(name)
            };
            if let Some(entry) = resolved {
                let binding = entry.binding.clone();
                map.bind_use(&expr.span, binding);
                for g in generics {
                    walk_type(g, scope, map)?;
                }
            } else if is_builtin_ctor_name(name) {
                // Built in constructor identifiers (`None`, `Some`, `Ok`,
                // `Err`), the `print` free function, and the
                // internal `__io_*` intrinsics bypass scope lookup. The
                // type checker recognizes them and assigns the correct
                // type at the call site.
                for g in generics {
                    walk_type(g, scope, map)?;
                }
            } else {
                return Err(RavenError::resolve(
                    ResolveError::UnresolvedName(name.clone()),
                    expr.span.clone(),
                ));
            }
        }
        ExprKind::StructLit {
            name,
            generics,
            fields,
        } => {
            // The struct name itself is a use site; if it isn't in
            // scope as a struct or import, raise UnresolvedName.
            let entry = scope.lookup(name).ok_or_else(|| {
                RavenError::resolve(
                    ResolveError::UnresolvedName(name.clone()),
                    expr.span.clone(),
                )
            })?;
            // The span attached to the StructLit covers the whole
            // literal, including the brace block. We record the
            // binding against that span; the type checker can subset
            // it later if needed.
            let binding = entry.binding.clone();
            map.bind_use(&expr.span, binding);
            for g in generics {
                walk_type(g, scope, map)?;
            }
            for f in fields {
                walk_expr(&f.value, scope, map)?;
            }
        }
        ExprKind::InterpolatedString(fragments) => {
            // Each embedded `${expr}` is a normal expression that may
            // reference names in scope at the literal's location. The
            // parser parsed each fragment against a synthetic source
            // path, so their use sites are bound under disjoint keys.
            for frag in fragments {
                if let crate::ast::StrFragment::Expr(e) = frag {
                    walk_expr(e, scope, map)?;
                }
            }
        }
        ExprKind::Array(items) | ExprKind::Tuple(items) | ExprKind::SetLit(items) => {
            for e in items {
                walk_expr(e, scope, map)?;
            }
        }
        ExprKind::MapLit(pairs) => {
            for (k, v) in pairs {
                walk_expr(k, scope, map)?;
                walk_expr(v, scope, map)?;
            }
        }
        ExprKind::Paren(e) => walk_expr(e, scope, map)?,
        ExprKind::Block(b) => walk_block(b, scope, map)?,
        ExprKind::Unary { operand, .. } => walk_expr(operand, scope, map)?,
        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, scope, map)?;
            walk_expr(rhs, scope, map)?;
        }
        ExprKind::Range { start, end, .. } => {
            walk_expr(start, scope, map)?;
            walk_expr(end, scope, map)?;
        }
        ExprKind::Call { callee, args } => {
            walk_expr(callee, scope, map)?;
            for a in args {
                walk_expr(a, scope, map)?;
            }
        }
        ExprKind::MethodCall {
            receiver,
            generics,
            args,
            ..
        } => {
            walk_expr(receiver, scope, map)?;
            for g in generics {
                walk_type(g, scope, map)?;
            }
            for a in args {
                walk_expr(a, scope, map)?;
            }
            // The method name itself is resolved by the type checker
            // once the receiver type is known. We deliberately do not
            // bind it here.
        }
        ExprKind::Field { receiver, .. } => walk_expr(receiver, scope, map)?,
        ExprKind::Index { receiver, index } => {
            walk_expr(receiver, scope, map)?;
            walk_expr(index, scope, map)?;
        }
        ExprKind::Try(e) => walk_expr(e, scope, map)?,
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            walk_expr(cond, scope, map)?;
            walk_block(then_branch, scope, map)?;
            if let Some(eb) = else_branch {
                match eb.as_ref() {
                    crate::ast::ElseBranch::If(e) => walk_expr(e, scope, map)?,
                    crate::ast::ElseBranch::Block(b) => walk_block(b, scope, map)?,
                }
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            walk_expr(scrutinee, scope, map)?;
            for arm in arms {
                scope.push(ScopeKind::Pattern);
                bind_pattern(&arm.pattern, scope)?;
                if let Some(g) = &arm.guard {
                    walk_expr(g, scope, map)?;
                }
                walk_expr(&arm.body, scope, map)?;
                scope.pop();
            }
        }
        ExprKind::Loop(b) => walk_block(b, scope, map)?,
        ExprKind::While { cond, body } => {
            walk_expr(cond, scope, map)?;
            walk_block(body, scope, map)?;
        }
        ExprKind::For {
            pattern,
            iter,
            body,
        } => {
            walk_expr(iter, scope, map)?;
            scope.push(ScopeKind::Pattern);
            bind_pattern(pattern, scope)?;
            walk_block(body, scope, map)?;
            scope.pop();
        }
        ExprKind::Lambda {
            params, ret, body, ..
        } => {
            scope.push(ScopeKind::Function);
            for p in params {
                insert_param(scope, &p.name, p.span.clone())?;
                if let Some(t) = &p.ty {
                    walk_type(t, scope, map)?;
                }
            }
            if let Some(r) = ret {
                walk_type(r, scope, map)?;
            }
            match body {
                LambdaBody::Block(b) => walk_block(b, scope, map)?,
                LambdaBody::Expr(e) => walk_expr(e, scope, map)?,
            }
            scope.pop();
        }
    }
    Ok(())
}

/// Bind every identifier introduced by `pattern` in the current scope
/// (which the caller is expected to push as `ScopeKind::Pattern`).
fn bind_pattern(pattern: &Pattern, scope: &mut ScopeStack) -> Result<(), RavenError> {
    match &pattern.kind {
        PatternKind::Wildcard | PatternKind::Literal(_) | PatternKind::Range { .. } => {}
        PatternKind::Ident(name) => {
            // An identifier in a pattern always binds a fresh name in
            // this scope. The resolver cannot tell at this point
            // whether the user intended to match an enum constructor
            // (`None`, `Some(x)`); the type checker reconciles that
            // later using the scrutinee's type.
            scope.insert_shadowing(
                name,
                Binding::PatternBinding(pattern.span.clone()),
                pattern.span.clone(),
            );
        }
        PatternKind::Tuple { elements, .. } => {
            for e in elements {
                bind_pattern(e, scope)?;
            }
        }
        PatternKind::Struct { fields, .. } => {
            for f in fields {
                if let Some(inner) = &f.pattern {
                    bind_pattern(inner, scope)?;
                } else {
                    // Shorthand `{ name }` introduces `name` directly.
                    scope.insert_shadowing(
                        &f.name,
                        Binding::PatternBinding(f.span.clone()),
                        f.span.clone(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn walk_type(ty: &Type, scope: &mut ScopeStack, map: &mut ResolutionMap) -> Result<(), RavenError> {
    match &ty.kind {
        TypeKind::Unit => Ok(()),
        TypeKind::Path(p) => walk_type_path(p, scope, map),
        TypeKind::Optional(inner) => walk_type(inner, scope, map),
        TypeKind::Dyn(p) => walk_type_path(p, scope, map),
        TypeKind::Function { params, ret } => {
            for p in params {
                walk_type(p, scope, map)?;
            }
            walk_type(ret, scope, map)
        }
    }
}

fn walk_type_path(
    path: &TypePath,
    scope: &mut ScopeStack,
    map: &mut ResolutionMap,
) -> Result<(), RavenError> {
    // The leading segment is a name we must resolve. Subsequent
    // segments are member lookups against the leading binding and are
    // the type checker's responsibility.
    let head = &path.segments[0];
    // `Self` is bound by an `impl` frame on the scope stack. If
    // present there it resolves to `SelfType`; otherwise the lookup
    // below raises `UnresolvedName`, which the dedicated check inside
    // `walk_expr` for `ExprKind::SelfUpper` already covers in
    // expression position.
    if head.name == "Self" {
        if let Some(entry) = scope.lookup("Self") {
            let binding = entry.binding.clone();
            map.bind_use(&head.span, binding);
            for g in &head.generics {
                walk_type(g, scope, map)?;
            }
            for seg in &path.segments[1..] {
                for g in &seg.generics {
                    walk_type(g, scope, map)?;
                }
            }
            return Ok(());
        }
        return Err(RavenError::resolve(
            ResolveError::SelfOutsideImpl,
            head.span.clone(),
        ));
    }

    // Built in primitive type names are unconditionally accepted; the
    // type checker is responsible for assigning them concrete kinds.
    if is_builtin_type_name(&head.name) {
        // Walk generic args anyway.
        for g in &head.generics {
            walk_type(g, scope, map)?;
        }
        for seg in &path.segments[1..] {
            for g in &seg.generics {
                walk_type(g, scope, map)?;
            }
        }
        return Ok(());
    }

    let entry = scope.lookup(&head.name).ok_or_else(|| {
        RavenError::resolve(
            ResolveError::UnresolvedName(head.name.clone()),
            head.span.clone(),
        )
    })?;
    let binding = entry.binding.clone();
    // A module-qualified type name (`net.TcpStream`) is not supported: a module
    // alias names no type. Report it here with a pointer to the selector import,
    // instead of accepting the alias and failing later as an opaque type error.
    if path.segments.len() > 1 && matches!(&binding, Binding::ImportAlias(_)) {
        let member = &path.segments[1].name;
        return Err(RavenError::resolve(
            ResolveError::Other(format!(
                "module-qualified type names like `{}.{member}` are not supported; \
                 import the type with a selector, for example `import std/<module> {{ {member} }}`",
                head.name
            )),
            head.span.clone(),
        ));
    }
    map.bind_use(&head.span, binding);
    for g in &head.generics {
        walk_type(g, scope, map)?;
    }
    for seg in &path.segments[1..] {
        for g in &seg.generics {
            walk_type(g, scope, map)?;
        }
    }
    Ok(())
}

/// Builtin type names known to the resolver. These bypass scope
/// lookup; the type checker assigns them their concrete meaning.
fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "Int"
            | "Float"
            | "Bool"
            | "String"
            | "Char"
            | "Unit"
            | "Array"
            | "Option"
            | "Result"
            | "Vec"
            | "List"
            | "CString"
            | "CStr"
            | "CInt"
            | "CLong"
            | "CSize"
            | "CFloat"
            | "CDouble"
            | "CPtr"
            | "CFnPtr"
            | "Any"
    )
}

/// Builtin constructor identifiers. These appear in expressions
/// (`None`, `Some(x)`, `Ok(v)`, `Err(e)`) and are recognized by the
/// type checker. The resolver bypasses scope lookup for them so they
/// can be used without an explicit import. `print` is treated the same
/// way: it is a built in free function intrinsic whose call sites are
/// wired to the runtime by the codegen back end.
///
/// The `__io_*` and `__str_*` names are internal stdlib intrinsics: the
/// bundled `std/io` and `std/string` sources call them to reach the
/// runtime's byte-level symbols. The leading `__` marks them internal;
/// users do not write them directly (they use the modules' exported
/// functions). They bypass scope lookup the same way the print builtins
/// do.
fn is_builtin_ctor_name(name: &str) -> bool {
    matches!(
        name,
        "None"
            | "Some"
            | "Ok"
            | "Err"
            | "print"
            | "type_name"
            | "field_names"
            | "field_types"
            | "variant_names"
            | "variant_field_types"
            | "to_any"
            | "cast"
            | "type_name_of"
            | "field_names_of"
            | "get_field"
            | "set_field"
            | "__ptr_alloc"
            | "__ptr_free"
            | "__ptr_load"
            | "__ptr_store"
            | "__ptr_offset"
            | "__ptr_is_null"
            | "__ptr_null"
            | "__io_print_str"
            | "__io_println_str"
            | "__io_read_line"
            | "__panic"
            | "__str_len"
            | "__str_byte_at"
            | "__str_substring"
            | "__str_from_byte"
            | "__str_concat"
    )
}
