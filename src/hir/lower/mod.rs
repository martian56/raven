//! The AST -> HIR lowering pass.
//!
//! Entry point: [`lower_file`] takes a [`TypedFile`] and returns a
//! [`HirProgram`]. Errors here are diagnostic-only; the type checker
//! is expected to have rejected programs that cannot be lowered.

pub mod expr;
pub mod pattern;
pub mod stmt;
pub mod sugar;

use std::cell::RefCell;

use crate::ast::{
    Decl, DeclKind, Function as AstFunction, FunctionBody, GenericParam, Impl, Param,
    Type as AstType, VariantPayload,
};
use crate::error::{RavenError, TypeError};
use crate::resolve::ResolvedFile;
use crate::span::Span;
use crate::tycheck::{
    collect::{collect_generic_params_for_owner, resolve_ty, scope_from_params, GenericScope},
    Ty, TypeEnv, TypedFile,
};

use super::decl::{
    HirEnum, HirExtern, HirExternFn, HirFn, HirImpl, HirItem, HirItemKind, HirStruct, HirTrait,
    HirVariant,
};
use super::HirProgram;

/// Lower a type-checked file into a `HirProgram`.
pub fn lower_file(typed: &TypedFile<'_>) -> Result<HirProgram, RavenError> {
    let cx = LowerCtx::new(typed.resolved, &typed.env, typed);
    let mut items = Vec::new();
    for decl in &typed.file.items {
        if let Some(item) = lower_decl(decl, &cx)? {
            items.push(item);
        }
    }
    Ok(HirProgram {
        items,
        span: typed.file.span.clone(),
    })
}

/// Context threaded through every lowering function. The fresh-name
/// counter sits behind a `RefCell` so the otherwise read-only context
/// can hand out unique identifiers without &mut self plumbing.
pub(crate) struct LowerCtx<'a> {
    pub resolved: &'a ResolvedFile<'a>,
    pub env: &'a TypeEnv,
    pub typed: &'a TypedFile<'a>,
    next_temp: RefCell<u32>,
}

impl<'a> LowerCtx<'a> {
    pub(crate) fn new(
        resolved: &'a ResolvedFile<'a>,
        env: &'a TypeEnv,
        typed: &'a TypedFile<'a>,
    ) -> Self {
        Self {
            resolved,
            env,
            typed,
            next_temp: RefCell::new(0),
        }
    }

    /// Allocate a fresh local name, prefixed with `__hir`. The names
    /// are namespaced so they cannot collide with user identifiers
    /// (Raven identifiers cannot start with `__hir`).
    pub(crate) fn fresh(&self, hint: &str) -> String {
        let mut n = self.next_temp.borrow_mut();
        let name = format!("__hir_{}_{}", hint, *n);
        *n += 1;
        name
    }

    /// Look up the type of an expression at the given span. Returns
    /// `Ty::Error` when no type was recorded (the type checker uses
    /// the same fallback, so this matches existing behavior).
    pub(crate) fn ty_at(&self, span: &Span) -> Ty {
        self.typed.types.lookup(span).cloned().unwrap_or(Ty::Error)
    }

    /// Resolved explicit type arguments recorded at a call's callee span,
    /// or an empty vector when the call wrote none.
    pub(crate) fn type_args_at(&self, span: &Span) -> Vec<Ty> {
        self.typed
            .types
            .lookup_type_args(span)
            .cloned()
            .unwrap_or_default()
    }

    /// True when the use site at `span` has no resolver binding. The
    /// reflection builtins (`type_name`, `field_names`) and `print` reach
    /// HIR as unbound identifiers; a user binding (an import or a local)
    /// would record a binding here, in which case the call is an ordinary
    /// one.
    pub(crate) fn is_unbound_builtin(&self, span: &Span) -> bool {
        self.resolved.map.lookup(span).is_none()
    }

    /// Resolve an identifier use site to the declared name of the
    /// top level function it binds to, if any.
    ///
    /// A bare call like `println(...)` carries the source spelling
    /// `println` at its callee span, but the binding may point at a
    /// function declared under a different name (for example a bundled
    /// stdlib function namespaced as `std.io.println`). The back end keys
    /// every call on the compiled function's name, so the call site must
    /// use that declared name rather than the source spelling. Returns
    /// `None` when the span does not resolve to a function (a local, a
    /// parameter, a builtin like `print`, ...), in which case the caller
    /// keeps the source spelling.
    pub(crate) fn fn_name_at(&self, span: &Span) -> Option<String> {
        use crate::resolve::Binding;
        match self.resolved.map.lookup(span)? {
            Binding::Function(decl_id) => {
                let decl = self.resolved.file.items.get(decl_id.0)?;
                if let crate::ast::DeclKind::Function(f) = &decl.kind {
                    Some(f.name.clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

fn lower_decl(decl: &Decl, cx: &LowerCtx<'_>) -> Result<Option<HirItem>, RavenError> {
    match &decl.kind {
        DeclKind::Function(f) => {
            let lowered = lower_function(f, None, &[], None, cx)?;
            Ok(Some(HirItem {
                span: decl.span.clone(),
                kind: HirItemKind::Function(lowered),
            }))
        }
        DeclKind::Struct(s) => {
            let params = collect_generic_params_for_owner(&s.generics, &s.span);
            let scope = scope_from_params(&params);
            let mut fields = Vec::with_capacity(s.fields.len());
            for f in &s.fields {
                let ty = resolve_ty_for_decl(&f.ty, cx, &scope)?;
                fields.push((f.name.clone(), ty, f.span.clone()));
            }
            Ok(Some(HirItem {
                span: decl.span.clone(),
                kind: HirItemKind::Struct(HirStruct {
                    name: s.name.clone(),
                    fields,
                    span: s.span.clone(),
                }),
            }))
        }
        DeclKind::Enum(e) => {
            let params = collect_generic_params_for_owner(&e.generics, &e.span);
            let scope = scope_from_params(&params);
            let mut variants = Vec::with_capacity(e.variants.len());
            for v in &e.variants {
                let fields = match &v.payload {
                    VariantPayload::Unit => Vec::new(),
                    VariantPayload::Tuple(tys) => {
                        let mut out = Vec::with_capacity(tys.len());
                        for (i, t) in tys.iter().enumerate() {
                            let ty = resolve_ty_for_decl(t, cx, &scope)?;
                            out.push((i.to_string(), ty, t.span.clone()));
                        }
                        out
                    }
                    VariantPayload::Struct(named) => {
                        let mut out = Vec::with_capacity(named.len());
                        for f in named {
                            let ty = resolve_ty_for_decl(&f.ty, cx, &scope)?;
                            out.push((f.name.clone(), ty, f.span.clone()));
                        }
                        out
                    }
                };
                variants.push(HirVariant {
                    name: v.name.clone(),
                    fields,
                    span: v.span.clone(),
                });
            }
            Ok(Some(HirItem {
                span: decl.span.clone(),
                kind: HirItemKind::Enum(HirEnum {
                    name: e.name.clone(),
                    variants,
                    span: e.span.clone(),
                }),
            }))
        }
        DeclKind::Trait(t) => {
            // A trait method's `Self` is abstract. The trait's HIR body is
            // never lowered to MIR (only concrete impl methods are), so a
            // placeholder `Self` type lets `self` receivers and any `Self`
            // annotations resolve without a concrete implementing type.
            let abstract_self = Ty::Error;
            let mut methods = Vec::with_capacity(t.members.len());
            for m in &t.members {
                methods.push(lower_function(
                    m,
                    Some(&abstract_self),
                    &t.generics,
                    Some(&t.span),
                    cx,
                )?);
            }
            Ok(Some(HirItem {
                span: decl.span.clone(),
                kind: HirItemKind::Trait(HirTrait {
                    name: t.name.clone(),
                    methods,
                    span: t.span.clone(),
                }),
            }))
        }
        DeclKind::Impl(i) => {
            let (self_ty, self_name) = impl_self_ty(i, cx)?;
            let trait_name = if i.for_type.is_some() {
                Some(path_first_name(&i.trait_or_type))
            } else {
                None
            };
            let mut methods = Vec::with_capacity(i.items.len());
            for m in &i.items {
                methods.push(lower_function(
                    m,
                    Some(&self_ty),
                    &i.generics,
                    Some(&i.span),
                    cx,
                )?);
            }
            Ok(Some(HirItem {
                span: decl.span.clone(),
                kind: HirItemKind::Impl(HirImpl {
                    self_name,
                    self_ty,
                    trait_name,
                    methods,
                    span: i.span.clone(),
                }),
            }))
        }
        DeclKind::Const(c) => {
            let scope = GenericScope::new();
            let ty = resolve_ty_for_decl(&c.ty, cx, &scope)?;
            let value = expr::lower_expr(&c.value, &ty, cx)?;
            Ok(Some(HirItem {
                span: decl.span.clone(),
                kind: HirItemKind::Const {
                    name: c.name.clone(),
                    ty,
                    value,
                },
            }))
        }
        DeclKind::Let(l) => {
            let scope = GenericScope::new();
            let ty = match &l.ty {
                Some(t) => resolve_ty_for_decl(t, cx, &scope)?,
                None => l
                    .init
                    .as_ref()
                    .map(|e| cx.ty_at(&e.span))
                    .unwrap_or(Ty::Error),
            };
            let init = match &l.init {
                Some(init) => Some(expr::lower_expr(init, &ty, cx)?),
                None => None,
            };
            Ok(Some(HirItem {
                span: decl.span.clone(),
                kind: HirItemKind::Let {
                    name: l.name.clone(),
                    ty,
                    init,
                },
            }))
        }
        DeclKind::Import(_) => Ok(Some(HirItem {
            span: decl.span.clone(),
            kind: HirItemKind::Opaque("import".into()),
        })),
        DeclKind::Extern(ext) => {
            // Resolve each foreign signature's parameter and return types
            // so codegen can declare the symbol with its C ABI shape.
            // Extern functions never have generic parameters, so an empty
            // scope is correct.
            let scope = scope_from_params(&[]);
            let mut items = Vec::with_capacity(ext.items.len());
            for item in &ext.items {
                let mut params = Vec::with_capacity(item.params.len());
                for p in &item.params {
                    params.push(resolve_ty_for_decl(&p.ty, cx, &scope)?);
                }
                let ret = match &item.ret {
                    Some(t) => resolve_ty_for_decl(t, cx, &scope)?,
                    None => Ty::Unit,
                };
                items.push(HirExternFn {
                    name: item.name.clone(),
                    params,
                    ret,
                    span: item.span.clone(),
                });
            }
            Ok(Some(HirItem {
                span: decl.span.clone(),
                kind: HirItemKind::Extern(HirExtern {
                    abi: ext.abi.clone(),
                    items,
                    span: ext.span.clone(),
                }),
            }))
        }
    }
}

fn lower_function(
    f: &AstFunction,
    self_ty: Option<&Ty>,
    extra_generics: &[GenericParam],
    extra_owner: Option<&Span>,
    cx: &LowerCtx<'_>,
) -> Result<HirFn, RavenError> {
    let mut sigs = Vec::new();
    if !extra_generics.is_empty() {
        // The enclosing impl or trait owns its generic parameters: their
        // owner is the impl/trait span, not the method span. This must
        // match how the implementing type (`impl_self_ty`) and the type
        // checker (`fill_impl`) resolve the same parameters, so a method
        // that returns the impl's `T` and the impl's `Self<T>` agree on
        // one `ParamId`. Falling back to the method span would mint a
        // distinct `T` per method and break monomorphization's
        // substitution.
        let owner = extra_owner.unwrap_or(&f.span);
        sigs.extend(collect_generic_params_for_owner(extra_generics, owner));
    }
    sigs.extend(collect_generic_params_for_owner(&f.generics, &f.span));
    let scope = scope_from_params(&sigs);

    let mut params = Vec::with_capacity(f.params.len());
    for p in &f.params {
        let ty = resolve_param_ty(p, cx, self_ty, &scope)?;
        params.push((p.name.clone(), ty, p.span.clone()));
    }
    let ret = match &f.ret {
        Some(t) => resolve_ty_for_decl_with_self(t, cx, self_ty, &scope)?,
        None => Ty::Unit,
    };
    let body = match &f.body {
        FunctionBody::Block(b) => Some(expr::lower_block_to_block(b, &ret, cx)?),
        FunctionBody::Expr(e) => Some(expr::lower_expr_as_block(e, &ret, cx)?),
        FunctionBody::None => None,
    };
    Ok(HirFn {
        name: f.name.clone(),
        params,
        ret,
        generics: sigs.iter().map(|s| s.id.clone()).collect(),
        body,
        span: f.span.clone(),
    })
}

fn resolve_param_ty(
    p: &Param,
    cx: &LowerCtx<'_>,
    self_ty: Option<&Ty>,
    scope: &GenericScope,
) -> Result<Ty, RavenError> {
    resolve_ty(&p.ty, cx.resolved, cx.env, self_ty, scope)
}

fn resolve_ty_for_decl(
    ty: &AstType,
    cx: &LowerCtx<'_>,
    scope: &GenericScope,
) -> Result<Ty, RavenError> {
    resolve_ty(ty, cx.resolved, cx.env, None, scope)
}

fn resolve_ty_for_decl_with_self(
    ty: &AstType,
    cx: &LowerCtx<'_>,
    self_ty: Option<&Ty>,
    scope: &GenericScope,
) -> Result<Ty, RavenError> {
    resolve_ty(ty, cx.resolved, cx.env, self_ty, scope)
}

fn impl_self_ty(i: &Impl, cx: &LowerCtx<'_>) -> Result<(Ty, String), RavenError> {
    let path = match &i.for_type {
        Some(t) => t,
        None => &i.trait_or_type,
    };
    let params = collect_generic_params_for_owner(&i.generics, &i.span);
    let scope = scope_from_params(&params);
    let ast_ty = AstType {
        kind: crate::ast::TypeKind::Path(path.clone()),
        span: path.span.clone(),
    };
    let ty = resolve_ty(&ast_ty, cx.resolved, cx.env, None, &scope)?;
    let name = path_first_name(path);
    Ok((ty, name))
}

fn path_first_name(path: &crate::ast::TypePath) -> String {
    path.segments
        .last()
        .map(|s| s.name.clone())
        .unwrap_or_default()
}

/// Convenience: lift an arbitrary diagnostic into a `RavenError::Type`.
/// HIR lowering should not normally emit errors (the type checker does
/// that job), but a few invariants are easier to assert than to thread
/// out at compile time.
#[allow(dead_code)]
pub(crate) fn ty_error(msg: impl Into<String>, span: &Span) -> RavenError {
    RavenError::ty(TypeError::Custom(msg.into()), span.clone())
}
