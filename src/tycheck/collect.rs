//! Declaration type collection pass.
//!
//! Walks every top level `Decl` and inserts its signature into the
//! [`TypeEnv`]. Function bodies are not inspected; only signatures and
//! shape (struct fields, enum variants, trait method signatures, impl
//! method signatures) are recorded.

use std::collections::HashMap;

use crate::ast::{
    DeclKind, Enum, Function, GenericParam, Impl, Struct, Trait, Type, TypeKind, TypePath,
    VariantPayload,
};
use crate::error::{RavenError, TypeError};
use crate::resolve::{Binding, DeclId, ResolvedFile};
use crate::span::Span;

use super::env::{
    EnumSig, FieldSig, FnSig, GenericParamSig, ImplSig, StructSig, TraitSig, TypeEnv,
    VariantPayloadSig, VariantSig,
};
use super::ty::{FfiTy, ParamId, Ty};

/// A small lexical scope of generic parameters introduced by an
/// enclosing declaration. Layered as a stack of frames so methods
/// inside an `impl` see both the impl's parameters and their own.
#[derive(Debug, Default, Clone)]
pub struct GenericScope {
    frames: Vec<HashMap<String, ParamId>>,
}

impl GenericScope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    pub fn insert(&mut self, name: &str, id: ParamId) {
        if let Some(top) = self.frames.last_mut() {
            top.insert(name.to_string(), id);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&ParamId> {
        for frame in self.frames.iter().rev() {
            if let Some(p) = frame.get(name) {
                return Some(p);
            }
        }
        None
    }
}

/// Walk `resolved` and populate `env` with every top level signature.
pub fn collect_declarations(
    resolved: &ResolvedFile<'_>,
    env: &mut TypeEnv,
) -> Result<(), RavenError> {
    let file = resolved.file;

    // First sub pass: gather struct, enum, and trait names so types
    // referenced by later signatures can be looked up regardless of
    // declaration order. The generic parameter lists are recorded here
    // so other signatures can refer to them via Ty::Param immediately.
    for (idx, decl) in file.items.iter().enumerate() {
        let id = DeclId(idx);
        match &decl.kind {
            DeclKind::Struct(s) => {
                let generics = collect_generic_params(&s.generics, &decl.span);
                env.structs.insert(
                    id,
                    StructSig {
                        name: s.name.clone(),
                        generics,
                        fields: Vec::new(),
                        span: s.span.clone(),
                    },
                );
            }
            DeclKind::Enum(e) => {
                let generics = collect_generic_params(&e.generics, &decl.span);
                env.enums.insert(
                    id,
                    EnumSig {
                        name: e.name.clone(),
                        generics,
                        variants: Vec::new(),
                        span: e.span.clone(),
                    },
                );
            }
            DeclKind::Trait(t) => {
                let generics = collect_generic_params(&t.generics, &decl.span);
                env.traits.insert(
                    id,
                    TraitSig {
                        name: t.name.clone(),
                        generics,
                        methods: HashMap::new(),
                        method_order: Vec::new(),
                        span: t.span.clone(),
                    },
                );
            }
            _ => {}
        }
    }

    // Second sub pass: now that every named type is known, fill in
    // signatures, field types, variant payloads, and trait methods.
    for (idx, decl) in file.items.iter().enumerate() {
        let id = DeclId(idx);
        match &decl.kind {
            DeclKind::Function(f) => {
                let mut scope = GenericScope::new();
                scope.push();
                let generics = collect_generic_params(&f.generics, &decl.span);
                push_generics_into_scope(&mut scope, &generics);
                let sig = collect_fn_sig(f, resolved, env, None, &scope, generics)?;
                scope.pop();
                env.functions.insert(id, sig);
            }
            DeclKind::Struct(s) => fill_struct(id, s, resolved, env)?,
            DeclKind::Enum(e) => fill_enum(id, e, resolved, env)?,
            DeclKind::Trait(t) => fill_trait(id, t, resolved, env)?,
            DeclKind::Impl(i) => fill_impl(i, resolved, env)?,
            DeclKind::Extern(ext) => {
                let scope = GenericScope::new();
                for (item_idx, item) in ext.items.iter().enumerate() {
                    let params = item
                        .params
                        .iter()
                        .map(|p| resolve_ty(&p.ty, resolved, env, None, &scope))
                        .collect::<Result<Vec<_>, _>>()?;
                    let ret = match &item.ret {
                        Some(t) => resolve_ty(t, resolved, env, None, &scope)?,
                        None => Ty::Unit,
                    };
                    env.externs.insert(
                        (id, item_idx),
                        FnSig {
                            name: item.name.clone(),
                            generics: Vec::new(),
                            params,
                            ret,
                            span: item.span.clone(),
                            has_self: false,
                        },
                    );
                }
            }
            DeclKind::Const(c) => {
                let scope = GenericScope::new();
                let ty = resolve_ty(&c.ty, resolved, env, None, &scope)?;
                env.consts.insert(id, ty);
            }
            DeclKind::Let(l) => {
                let scope = GenericScope::new();
                let ty = match &l.ty {
                    Some(t) => resolve_ty(t, resolved, env, None, &scope)?,
                    None => Ty::Error,
                };
                env.statics.insert(id, ty);
            }
            DeclKind::Import(_) => {}
        }
    }

    // Third sub pass: now that every signature is recorded, resolve
    // trait bound names on generic parameters. Bounds are stored by
    // trait name; the resolver has already validated that the path
    // resolves to a trait, so all we need is the head segment's name.
    Ok(())
}

/// Build a list of [`GenericParamSig`] from a parsed generic parameter
/// list. The trait bounds are captured by name (the resolver already
/// validated that they point at trait declarations).
fn collect_generic_params(params: &[GenericParam], owner: &Span) -> Vec<GenericParamSig> {
    let mut sigs: Vec<GenericParamSig> = params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let bounds = p
                .bounds
                .iter()
                .map(|b| {
                    // The head segment name is the trait's short name.
                    b.segments
                        .last()
                        .map(|s| s.name.clone())
                        .unwrap_or_default()
                })
                .collect();
            GenericParamSig {
                id: ParamId::new(owner, i, p.name.clone()),
                bounds,
                // Filled below once every sibling parameter id is known.
                bound_args: Vec::new(),
                span: p.span.clone(),
            }
        })
        .collect();
    // Resolve the type arguments of each bound against a scope built from
    // the sibling parameters in this same list. A bound such as
    // `S: Iterator<T>` references the sibling `T`, so the scope must hold
    // every parameter before any bound is resolved. Arguments that are
    // bare parameter names or built-in primitives resolve without needing
    // the type environment; anything else (an unresolvable argument) is
    // recorded as `Ty::Error` and simply leaves the trait parameter
    // abstract, which is the prior behavior.
    let mut scope = GenericScope::new();
    scope.push();
    push_generics_into_scope(&mut scope, &sigs);
    for (i, p) in params.iter().enumerate() {
        let mut per_bound: Vec<Vec<Ty>> = Vec::with_capacity(p.bounds.len());
        for b in &p.bounds {
            let seg_args = b.segments.last().map(|s| &s.generics);
            let args = match seg_args {
                Some(gens) => gens
                    .iter()
                    .map(|g| resolve_bound_arg(g, &scope))
                    .collect::<Vec<_>>(),
                None => Vec::new(),
            };
            per_bound.push(args);
        }
        sigs[i].bound_args = per_bound;
    }
    sigs
}

/// Resolve one type argument of a trait bound to a [`Ty`] using only the
/// lexical scope of the surrounding generic parameter list. Parameter
/// names and built-in primitives resolve directly; anything requiring a
/// type-environment lookup is left as `Ty::Error` so the caller falls
/// back to leaving the trait parameter abstract.
fn resolve_bound_arg(ty: &Type, scope: &GenericScope) -> Ty {
    match &ty.kind {
        TypeKind::Path(p) => {
            let head = &p.segments[0];
            if let Some(id) = scope.lookup(&head.name) {
                return Ty::Param(id.clone());
            }
            match head.name.as_str() {
                "Int" => Ty::Int,
                "Float" => Ty::Float,
                "Bool" => Ty::Bool,
                "Char" => Ty::Char,
                "String" => Ty::Str,
                "Unit" => Ty::Unit,
                _ => Ty::Error,
            }
        }
        TypeKind::Unit => Ty::Unit,
        _ => Ty::Error,
    }
}

fn push_generics_into_scope(scope: &mut GenericScope, params: &[GenericParamSig]) {
    for p in params {
        scope.insert(&p.id.name, p.id.clone());
    }
}

fn fill_struct(
    id: DeclId,
    s: &Struct,
    resolved: &ResolvedFile<'_>,
    env: &mut TypeEnv,
) -> Result<(), RavenError> {
    let mut scope = GenericScope::new();
    scope.push();
    let entry_generics = env
        .structs
        .get(&id)
        .map(|sig| sig.generics.clone())
        .unwrap_or_default();
    push_generics_into_scope(&mut scope, &entry_generics);

    let mut fields = Vec::with_capacity(s.fields.len());
    for f in &s.fields {
        let ty = resolve_ty(&f.ty, resolved, env, None, &scope)?;
        fields.push(FieldSig {
            name: f.name.clone(),
            ty,
            span: f.span.clone(),
        });
    }
    let entry = env.structs.get_mut(&id).expect("struct sig pre populated");
    entry.fields = fields;
    Ok(())
}

fn fill_enum(
    id: DeclId,
    e: &Enum,
    resolved: &ResolvedFile<'_>,
    env: &mut TypeEnv,
) -> Result<(), RavenError> {
    let mut scope = GenericScope::new();
    scope.push();
    let entry_generics = env
        .enums
        .get(&id)
        .map(|sig| sig.generics.clone())
        .unwrap_or_default();
    push_generics_into_scope(&mut scope, &entry_generics);

    let mut variants = Vec::with_capacity(e.variants.len());
    for v in &e.variants {
        let payload = match &v.payload {
            VariantPayload::Unit => VariantPayloadSig::Unit,
            VariantPayload::Tuple(tys) => {
                let mut out = Vec::with_capacity(tys.len());
                for t in tys {
                    out.push(resolve_ty(t, resolved, env, None, &scope)?);
                }
                VariantPayloadSig::Tuple(out)
            }
            VariantPayload::Struct(fields) => {
                let mut out = Vec::with_capacity(fields.len());
                for f in fields {
                    out.push(FieldSig {
                        name: f.name.clone(),
                        ty: resolve_ty(&f.ty, resolved, env, None, &scope)?,
                        span: f.span.clone(),
                    });
                }
                VariantPayloadSig::Struct(out)
            }
        };
        variants.push(VariantSig {
            name: v.name.clone(),
            payload,
            span: v.span.clone(),
        });
    }
    let entry = env.enums.get_mut(&id).expect("enum sig pre populated");
    entry.variants = variants;
    Ok(())
}

fn fill_trait(
    id: DeclId,
    t: &Trait,
    resolved: &ResolvedFile<'_>,
    env: &mut TypeEnv,
) -> Result<(), RavenError> {
    let mut scope = GenericScope::new();
    scope.push();
    let trait_generics = env
        .traits
        .get(&id)
        .map(|sig| sig.generics.clone())
        .unwrap_or_default();
    push_generics_into_scope(&mut scope, &trait_generics);

    // Inside a trait declaration `Self` denotes the (not yet known)
    // implementing type. It resolves to an abstract `Ty::SelfTy(Error)`,
    // the same placeholder the `self` receiver already carries here. The
    // bound-driven method dispatch substitutes every `SelfTy` in a trait
    // method signature with the concrete receiver type at the call site,
    // so `equals(self, other: Self)` on a `T: Eq` with `T = Int` checks
    // `other` against `Int`.
    let trait_self = Ty::Error;
    let mut methods = HashMap::new();
    let mut method_order = Vec::with_capacity(t.members.len());
    for member in &t.members {
        scope.push();
        let m_generics = collect_generic_params(&member.generics, &member.span);
        push_generics_into_scope(&mut scope, &m_generics);
        let sig = collect_fn_sig(member, resolved, env, Some(&trait_self), &scope, m_generics)?;
        scope.pop();
        method_order.push(member.name.clone());
        methods.insert(member.name.clone(), sig);
    }
    let entry = env.traits.get_mut(&id).expect("trait sig pre populated");
    entry.methods = methods;
    entry.method_order = method_order;
    Ok(())
}

/// Report whether a trait is object-safe and, if not, why. A trait is
/// object-safe in this subset when none of its methods are generic and
/// none take `Self` by value in a non-receiver parameter position. The
/// returned `Err` string is a user-facing reason for the diagnostic.
pub fn object_safety_violation(sig: &TraitSig) -> Option<String> {
    for name in &sig.method_order {
        let Some(m) = sig.methods.get(name) else {
            continue;
        };
        if !m.generics.is_empty() {
            return Some(format!(
                "method `{}` is generic; methods with type parameters are not object-safe",
                name
            ));
        }
        // The receiver `self` is `Ty::SelfTy(..)` and is allowed. Any
        // other parameter that mentions `Self` by value is not object
        // safe in this subset.
        for (i, p) in m.params.iter().enumerate() {
            let is_receiver = i == 0 && matches!(p, Ty::SelfTy(_));
            if !is_receiver && ty_mentions_self(p) {
                return Some(format!(
                    "method `{}` takes `Self` by value in a non-receiver position, which is not object-safe",
                    name
                ));
            }
        }
    }
    None
}

/// Whether a resolved type mentions the `Self` type anywhere.
fn ty_mentions_self(ty: &Ty) -> bool {
    match ty {
        Ty::SelfTy(_) => true,
        Ty::Option(t) | Ty::List(t) => ty_mentions_self(t),
        Ty::Result(a, b) => ty_mentions_self(a) || ty_mentions_self(b),
        Ty::Struct { args, .. } | Ty::Enum { args, .. } => args.iter().any(ty_mentions_self),
        Ty::Function { params, ret } => {
            params.iter().any(ty_mentions_self) || ty_mentions_self(ret)
        }
        _ => false,
    }
}

fn fill_impl(i: &Impl, resolved: &ResolvedFile<'_>, env: &mut TypeEnv) -> Result<(), RavenError> {
    let mut scope = GenericScope::new();
    scope.push();
    let impl_generics = collect_generic_params(&i.generics, &i.span);
    push_generics_into_scope(&mut scope, &impl_generics);

    // The implementing type is `for_type` for trait impls; otherwise
    // `trait_or_type` itself.
    let (impl_path, trait_name) = match &i.for_type {
        Some(target) => {
            let trait_name = type_path_name(&i.trait_or_type);
            (target, Some(trait_name))
        }
        None => (&i.trait_or_type, None),
    };
    let self_ty = resolve_type_path(impl_path, resolved, env, None, &scope)?;

    let mut methods = HashMap::new();
    for f in &i.items {
        scope.push();
        let m_generics = collect_generic_params(&f.generics, &f.span);
        push_generics_into_scope(&mut scope, &m_generics);
        let sig = collect_fn_sig(f, resolved, env, Some(&self_ty), &scope, m_generics)?;
        scope.pop();
        methods.insert(f.name.clone(), sig);
    }
    env.impls.push(ImplSig {
        generics: impl_generics,
        self_ty,
        trait_name,
        methods,
        span: i.span.clone(),
    });
    Ok(())
}

fn collect_fn_sig(
    f: &Function,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    self_ty: Option<&Ty>,
    scope: &GenericScope,
    generics: Vec<GenericParamSig>,
) -> Result<FnSig, RavenError> {
    let mut params = Vec::with_capacity(f.params.len());
    let mut has_self = false;
    for p in &f.params {
        if p.name == "self" {
            has_self = true;
            let inner = self_ty.cloned().unwrap_or(Ty::Error);
            params.push(Ty::SelfTy(Box::new(inner)));
        } else {
            params.push(resolve_ty(&p.ty, resolved, env, self_ty, scope)?);
        }
    }
    let ret = match &f.ret {
        Some(t) => resolve_ty(t, resolved, env, self_ty, scope)?,
        None => Ty::Unit,
    };
    Ok(FnSig {
        name: f.name.clone(),
        generics,
        params,
        ret,
        span: f.span.clone(),
        has_self,
    })
}

/// Resolve an AST `Type` to a `Ty`. `self_ty` is the implementing
/// type when this resolution happens inside an impl block, used to
/// substitute `Self`. `scope` carries the enclosing generic parameters
/// so type paths like `T` resolve to `Ty::Param(...)`.
pub fn resolve_ty(
    ty: &Type,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    self_ty: Option<&Ty>,
    scope: &GenericScope,
) -> Result<Ty, RavenError> {
    match &ty.kind {
        TypeKind::Unit => Ok(Ty::Unit),
        TypeKind::Optional(inner) => {
            let t = resolve_ty(inner, resolved, env, self_ty, scope)?;
            Ok(Ty::Option(Box::new(t)))
        }
        TypeKind::Path(p) => resolve_type_path(p, resolved, env, self_ty, scope),
        TypeKind::Dyn(p) => resolve_dyn(p, resolved, env),
        TypeKind::Function { params, ret } => {
            let mut ps = Vec::with_capacity(params.len());
            for p in params {
                ps.push(resolve_ty(p, resolved, env, self_ty, scope)?);
            }
            let r = resolve_ty(ret, resolved, env, self_ty, scope)?;
            Ok(Ty::Function {
                params: ps,
                ret: Box::new(r),
            })
        }
    }
}

/// Resolve a `dyn Trait` type expression. The head segment must bind to
/// a trait declaration; the trait must be object-safe. The produced
/// [`Ty::Dyn`] carries the trait's method order so later passes lay out
/// the vtable in a stable slot order.
fn resolve_dyn(
    path: &TypePath,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
) -> Result<Ty, RavenError> {
    let head = &path.segments[0];
    let name = &head.name;
    let binding = resolved
        .map
        .lookup(&head.span)
        .ok_or_else(|| RavenError::ty(TypeError::UnknownType(name.clone()), head.span.clone()))?;
    let trait_id = match binding {
        Binding::Trait(id) => *id,
        _ => {
            return Err(RavenError::ty(
                TypeError::Custom(format!(
                    "`dyn {}` requires a trait; `{}` is not a trait",
                    name, name
                )),
                head.span.clone(),
            ));
        }
    };
    let sig = env
        .traits
        .get(&trait_id)
        .ok_or_else(|| RavenError::ty(TypeError::UnknownType(name.clone()), head.span.clone()))?;
    if let Some(reason) = object_safety_violation(sig) {
        return Err(RavenError::ty(
            TypeError::Custom(format!(
                "`dyn {}` is not allowed: {}. Consider a generic bound `<T: {}>` instead",
                name, reason, name
            )),
            head.span.clone(),
        ));
    }
    Ok(Ty::Dyn {
        name: sig.name.clone(),
        methods: sig.method_order.clone(),
    })
}

fn resolve_type_path(
    path: &TypePath,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    self_ty: Option<&Ty>,
    scope: &GenericScope,
) -> Result<Ty, RavenError> {
    let head = &path.segments[0];
    let name = &head.name;

    // Lexical generic parameter takes precedence over everything else.
    if let Some(param_id) = scope.lookup(name) {
        if !head.generics.is_empty() {
            return Err(RavenError::ty(
                TypeError::Custom(format!(
                    "`{}` is a type parameter; it cannot take type arguments",
                    name
                )),
                head.span.clone(),
            ));
        }
        return Ok(Ty::Param(param_id.clone()));
    }

    // Built in primitives and built in generics.
    match name.as_str() {
        "Int" => return ok_zero_generics(head, Ty::Int),
        "Float" => return ok_zero_generics(head, Ty::Float),
        "Bool" => return ok_zero_generics(head, Ty::Bool),
        "Char" => return ok_zero_generics(head, Ty::Char),
        "String" => return ok_zero_generics(head, Ty::Str),
        "Unit" => return ok_zero_generics(head, Ty::Unit),
        "Option" => {
            let inner = expect_one_generic(head, resolved, env, self_ty, scope)?;
            return Ok(Ty::Option(Box::new(inner)));
        }
        "Result" => {
            let (t, e) = expect_two_generics(head, resolved, env, self_ty, scope)?;
            return Ok(Ty::Result(Box::new(t), Box::new(e)));
        }
        "List" | "Array" | "Vec" => {
            let inner = expect_one_generic(head, resolved, env, self_ty, scope)?;
            return Ok(Ty::List(Box::new(inner)));
        }
        "CInt" => return ok_zero_generics(head, Ty::Ffi(FfiTy::CInt)),
        "CLong" => return ok_zero_generics(head, Ty::Ffi(FfiTy::CLong)),
        "CSize" => return ok_zero_generics(head, Ty::Ffi(FfiTy::CSize)),
        // `CStr` is the spec name; `CString` is accepted as an alias so
        // older `extern` signatures keep checking. Both denote a pointer
        // to a null-terminated byte buffer.
        "CStr" | "CString" => return ok_zero_generics(head, Ty::Ffi(FfiTy::CStr)),
        "CFloat" => return ok_zero_generics(head, Ty::Ffi(FfiTy::CFloat)),
        "CDouble" => return ok_zero_generics(head, Ty::Ffi(FfiTy::CDouble)),
        "CFnPtr" => return ok_zero_generics(head, Ty::Ffi(FfiTy::CFnPtr)),
        "CPtr" => {
            let inner = expect_one_generic(head, resolved, env, self_ty, scope)?;
            return Ok(Ty::Ffi(FfiTy::CPtr(Box::new(inner))));
        }
        "Self" => {
            return self_ty
                .cloned()
                .map(|t| Ty::SelfTy(Box::new(t)))
                .ok_or_else(|| {
                    RavenError::ty(
                        TypeError::Custom("`Self` used outside an impl block".into()),
                        head.span.clone(),
                    )
                });
        }
        _ => {}
    }

    // User declared types. The resolver records the head segment under
    // its span in the resolution map.
    let binding = resolved
        .map
        .lookup(&head.span)
        .ok_or_else(|| RavenError::ty(TypeError::UnknownType(name.clone()), head.span.clone()))?;
    match binding {
        Binding::Struct(id) => {
            let s = env.structs.get(id).ok_or_else(|| {
                RavenError::ty(TypeError::UnknownType(name.clone()), head.span.clone())
            })?;
            let expected = s.generics.len();
            let provided = head.generics.len();
            // Allow zero explicit args even when declaration has generics:
            // the type checker will instantiate fresh inference vars.
            if provided != 0 && provided != expected {
                return Err(RavenError::ty(
                    TypeError::GenericArityMismatch {
                        decl: s.name.clone(),
                        expected,
                        actual: provided,
                    },
                    head.span.clone(),
                ));
            }
            let mut args = Vec::with_capacity(provided);
            for g in &head.generics {
                args.push(resolve_ty(g, resolved, env, self_ty, scope)?);
            }
            Ok(Ty::Struct {
                id: *id,
                name: s.name.clone(),
                args,
            })
        }
        Binding::Enum(id) => {
            let e = env.enums.get(id).ok_or_else(|| {
                RavenError::ty(TypeError::UnknownType(name.clone()), head.span.clone())
            })?;
            let expected = e.generics.len();
            let provided = head.generics.len();
            if provided != 0 && provided != expected {
                return Err(RavenError::ty(
                    TypeError::GenericArityMismatch {
                        decl: e.name.clone(),
                        expected,
                        actual: provided,
                    },
                    head.span.clone(),
                ));
            }
            let mut args = Vec::with_capacity(provided);
            for g in &head.generics {
                args.push(resolve_ty(g, resolved, env, self_ty, scope)?);
            }
            Ok(Ty::Enum {
                id: *id,
                name: e.name.clone(),
                args,
            })
        }
        Binding::Trait(_) => Err(RavenError::ty(
            TypeError::Custom(format!(
                "`{}` is a trait; bare trait types are not yet supported (use `dyn Trait` in a future release)",
                name
            )),
            head.span.clone(),
        )),
        Binding::SelfType => self_ty
            .cloned()
            .map(|t| Ty::SelfTy(Box::new(t)))
            .ok_or_else(|| {
                RavenError::ty(
                    TypeError::Custom("`Self` used outside an impl block".into()),
                    head.span.clone(),
                )
            }),
        Binding::GenericParam { owner, name } => {
            // The resolver bound this name; locate its declared index
            // by name within the owner. The body checker carries the
            // same map; here we just construct a ParamId from the
            // resolver's identifier pair. Index discovery requires the
            // declaration index in the owner; we approximate by scanning
            // every signature for a matching ParamId by name + owner.
            // In practice the lexical scope above already handled the
            // common case, so this branch is the fallback.
            // Locate the matching ParamId by walking signatures.
            for sig in env.functions.values() {
                for p in &sig.generics {
                    if p.id.name == *name
                        && p.id.owner_start == owner.start
                        && p.id.owner_end == owner.end
                        && *p.id.owner_file == *owner.file
                    {
                        return Ok(Ty::Param(p.id.clone()));
                    }
                }
            }
            for sig in env.structs.values() {
                for p in &sig.generics {
                    if p.id.name == *name
                        && p.id.owner_start == owner.start
                        && p.id.owner_end == owner.end
                        && *p.id.owner_file == *owner.file
                    {
                        return Ok(Ty::Param(p.id.clone()));
                    }
                }
            }
            for sig in env.enums.values() {
                for p in &sig.generics {
                    if p.id.name == *name
                        && p.id.owner_start == owner.start
                        && p.id.owner_end == owner.end
                        && *p.id.owner_file == *owner.file
                    {
                        return Ok(Ty::Param(p.id.clone()));
                    }
                }
            }
            for sig in env.traits.values() {
                for p in &sig.generics {
                    if p.id.name == *name
                        && p.id.owner_start == owner.start
                        && p.id.owner_end == owner.end
                        && *p.id.owner_file == *owner.file
                    {
                        return Ok(Ty::Param(p.id.clone()));
                    }
                }
            }
            for sig in env.impls.iter() {
                for p in &sig.generics {
                    if p.id.name == *name
                        && p.id.owner_start == owner.start
                        && p.id.owner_end == owner.end
                        && *p.id.owner_file == *owner.file
                    {
                        return Ok(Ty::Param(p.id.clone()));
                    }
                }
                for sig in sig.methods.values() {
                    for p in &sig.generics {
                        if p.id.name == *name
                            && p.id.owner_start == owner.start
                            && p.id.owner_end == owner.end
                            && *p.id.owner_file == *owner.file
                        {
                            return Ok(Ty::Param(p.id.clone()));
                        }
                    }
                }
            }
            // Fallback: build a fresh ParamId from the binding. The
            // owner span uniquely identifies the declaration; the index
            // is unknown, so we use 0. Two distinct parameters on the
            // same declaration could in theory collide here, but the
            // lexical scope path above already handles correctly bound
            // uses in well formed programs.
            Ok(Ty::Param(ParamId::new(owner, 0, name.clone())))
        }
        _ => Err(RavenError::ty(
            TypeError::UnknownType(name.clone()),
            head.span.clone(),
        )),
    }
}

fn ok_zero_generics(seg: &crate::ast::TypePathSegment, ty: Ty) -> Result<Ty, RavenError> {
    if !seg.generics.is_empty() {
        return Err(RavenError::ty(
            TypeError::Custom(format!("`{}` does not take generic arguments", seg.name)),
            seg.span.clone(),
        ));
    }
    Ok(ty)
}

fn expect_one_generic(
    seg: &crate::ast::TypePathSegment,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    self_ty: Option<&Ty>,
    scope: &GenericScope,
) -> Result<Ty, RavenError> {
    if seg.generics.len() != 1 {
        return Err(RavenError::ty(
            TypeError::Custom(format!(
                "`{}` takes exactly one type argument, got {}",
                seg.name,
                seg.generics.len()
            )),
            seg.span.clone(),
        ));
    }
    resolve_ty(&seg.generics[0], resolved, env, self_ty, scope)
}

fn expect_two_generics(
    seg: &crate::ast::TypePathSegment,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    self_ty: Option<&Ty>,
    scope: &GenericScope,
) -> Result<(Ty, Ty), RavenError> {
    if seg.generics.len() != 2 {
        return Err(RavenError::ty(
            TypeError::Custom(format!(
                "`{}` takes exactly two type arguments, got {}",
                seg.name,
                seg.generics.len()
            )),
            seg.span.clone(),
        ));
    }
    let a = resolve_ty(&seg.generics[0], resolved, env, self_ty, scope)?;
    let b = resolve_ty(&seg.generics[1], resolved, env, self_ty, scope)?;
    Ok((a, b))
}

fn type_path_name(path: &TypePath) -> String {
    path.segments
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

/// Build a fresh `GenericScope` containing the parameters of the
/// declaration named by `decl_span` and `kind`. Helper used by callers
/// outside `collect.rs` that need to resolve a type expression in the
/// context of a particular declaration.
pub fn scope_from_params(params: &[GenericParamSig]) -> GenericScope {
    let mut s = GenericScope::new();
    s.push();
    push_generics_into_scope(&mut s, params);
    s
}

/// Public helper: layer additional parameters onto an existing scope.
pub fn push_into_scope(scope: &mut GenericScope, params: &[GenericParamSig]) {
    scope.push();
    push_generics_into_scope(scope, params);
}

/// Public helper: build a [`GenericParamSig`] list from an AST generic
/// list against a specific owner span.
pub fn collect_generic_params_for_owner(
    params: &[GenericParam],
    owner: &Span,
) -> Vec<GenericParamSig> {
    collect_generic_params(params, owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::parse;
    use crate::resolve::{resolve_file, LoadedSource, SourceLoader};
    use std::path::{Path, PathBuf};

    struct NoLoader;
    impl SourceLoader for NoLoader {
        fn load(&mut self, _i: &Path, _t: &str) -> Option<LoadedSource> {
            None
        }
    }

    fn check_source(src: &str) -> Result<TypeEnv, RavenError> {
        let tokens = Lexer::new(src.to_string(), PathBuf::from("t.rv"))
            .tokenize()
            .expect("lex");
        let file = parse(&tokens).expect("parse");
        let mut loader = NoLoader;
        let resolved = resolve_file(&file, &mut loader)?;
        let mut env = TypeEnv::new();
        collect_declarations(&resolved, &mut env)?;
        Ok(env)
    }

    #[test]
    fn collects_function_signature() {
        let env = check_source("fun add(a: Int, b: Int) -> Int = a + b\n").unwrap();
        let sig = env.functions.values().next().expect("one function");
        assert_eq!(sig.params, vec![Ty::Int, Ty::Int]);
        assert_eq!(sig.ret, Ty::Int);
    }

    #[test]
    fn collects_struct_fields() {
        let env = check_source("struct Point { x: Int, y: Int }\n").unwrap();
        let s = env.structs.values().next().expect("one struct");
        assert_eq!(s.fields.len(), 2);
        assert_eq!(s.fields[0].name, "x");
        assert_eq!(s.fields[0].ty, Ty::Int);
    }

    #[test]
    fn collects_generic_function_signature() {
        let env = check_source("fun id<T>(x: T) -> T = x\n").unwrap();
        let sig = env.functions.values().next().expect("one function");
        assert_eq!(sig.generics.len(), 1);
        assert_eq!(sig.generics[0].id.name, "T");
        match &sig.params[0] {
            Ty::Param(p) => assert_eq!(p.name, "T"),
            other => panic!("expected Ty::Param, got {:?}", other),
        }
    }

    #[test]
    fn collects_generic_struct_field_types() {
        let env = check_source("struct Box<T> { value: T }\n").unwrap();
        let s = env.structs.values().next().expect("one struct");
        assert_eq!(s.generics.len(), 1);
        match &s.fields[0].ty {
            Ty::Param(p) => assert_eq!(p.name, "T"),
            other => panic!("expected Ty::Param, got {:?}", other),
        }
    }

    #[test]
    fn option_type_path_resolves() {
        let env = check_source("fun takes(x: Option<Int>) -> Int = 0\n").unwrap();
        let sig = env.functions.values().next().unwrap();
        assert_eq!(sig.params, vec![Ty::Option(Box::new(Ty::Int))]);
    }

    #[test]
    fn result_type_path_resolves() {
        let env = check_source("fun work(x: Result<Int, String>) -> Int = 0\n").unwrap();
        let sig = env.functions.values().next().unwrap();
        assert_eq!(
            sig.params,
            vec![Ty::Result(Box::new(Ty::Int), Box::new(Ty::Str))]
        );
    }

    #[test]
    fn struct_and_impl_collected() {
        let env = check_source(
            "struct Point { x: Int }\nimpl Point { fun get_x(self) -> Int = self.x }\n",
        )
        .unwrap();
        assert_eq!(env.structs.len(), 1);
        assert_eq!(env.impls.len(), 1);
        let i = &env.impls[0];
        assert!(i.trait_name.is_none());
        assert!(i.methods.contains_key("get_x"));
    }

    #[test]
    fn enum_variants_collected() {
        let env = check_source("enum Color { Red, Green, Rgb(Int, Int, Int) }\n").unwrap();
        let e = env.enums.values().next().expect("one enum");
        assert_eq!(e.variants.len(), 3);
        assert_eq!(e.variants[0].name, "Red");
        assert!(matches!(e.variants[2].payload, VariantPayloadSig::Tuple(_)));
    }
}
