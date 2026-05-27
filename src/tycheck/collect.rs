//! Declaration type collection pass.
//!
//! Walks every top level `Decl` and inserts its signature into the
//! [`TypeEnv`]. Function bodies are not inspected; only signatures and
//! shape (struct fields, enum variants, trait method signatures, impl
//! method signatures) are recorded.

use std::collections::HashMap;

use crate::ast::{
    Decl, DeclKind, Enum, Function, Impl, Struct, Trait, Type, TypeKind, TypePath, VariantPayload,
};
use crate::error::{RavenError, TypeError};
use crate::resolve::{Binding, DeclId, ResolvedFile};

use super::env::{
    EnumSig, FieldSig, FnSig, ImplSig, StructSig, TraitSig, TypeEnv, VariantPayloadSig, VariantSig,
};
use super::ty::Ty;

/// Walk `resolved` and populate `env` with every top level signature.
pub fn collect_declarations(
    resolved: &ResolvedFile<'_>,
    env: &mut TypeEnv,
) -> Result<(), RavenError> {
    let file = resolved.file;

    // First sub pass: gather struct and enum names so types referenced
    // by later signatures can be looked up regardless of declaration
    // order.
    for (idx, decl) in file.items.iter().enumerate() {
        let id = DeclId(idx);
        match &decl.kind {
            DeclKind::Struct(s) => {
                reject_user_generics(&s.generics, decl, "structs")?;
                env.structs.insert(
                    id,
                    StructSig {
                        name: s.name.clone(),
                        fields: Vec::new(),
                        span: s.span.clone(),
                    },
                );
            }
            DeclKind::Enum(e) => {
                reject_user_generics(&e.generics, decl, "enums")?;
                env.enums.insert(
                    id,
                    EnumSig {
                        name: e.name.clone(),
                        variants: Vec::new(),
                        span: e.span.clone(),
                    },
                );
            }
            DeclKind::Trait(t) => {
                reject_user_generics(&t.generics, decl, "traits")?;
                env.traits.insert(
                    id,
                    TraitSig {
                        name: t.name.clone(),
                        methods: HashMap::new(),
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
                reject_user_generics(&f.generics, decl, "functions")?;
                let sig = collect_fn_sig(f, resolved, env, None)?;
                env.functions.insert(id, sig);
            }
            DeclKind::Struct(s) => fill_struct(id, s, resolved, env)?,
            DeclKind::Enum(e) => fill_enum(id, e, resolved, env)?,
            DeclKind::Trait(t) => fill_trait(id, t, resolved, env)?,
            DeclKind::Impl(i) => fill_impl(i, resolved, env)?,
            DeclKind::Extern(ext) => {
                for (item_idx, item) in ext.items.iter().enumerate() {
                    let params = item
                        .params
                        .iter()
                        .map(|p| resolve_ty(&p.ty, resolved, env, None))
                        .collect::<Result<Vec<_>, _>>()?;
                    let ret = match &item.ret {
                        Some(t) => resolve_ty(t, resolved, env, None)?,
                        None => Ty::Unit,
                    };
                    env.externs.insert(
                        (id, item_idx),
                        FnSig {
                            name: item.name.clone(),
                            params,
                            ret,
                            span: item.span.clone(),
                            has_self: false,
                        },
                    );
                }
            }
            DeclKind::Const(c) => {
                let ty = resolve_ty(&c.ty, resolved, env, None)?;
                env.consts.insert(id, ty);
            }
            DeclKind::Let(l) => {
                let ty = match &l.ty {
                    Some(t) => resolve_ty(t, resolved, env, None)?,
                    None => Ty::Error,
                };
                env.statics.insert(id, ty);
            }
            DeclKind::Import(_) => {}
        }
    }
    Ok(())
}

fn fill_struct(
    id: DeclId,
    s: &Struct,
    resolved: &ResolvedFile<'_>,
    env: &mut TypeEnv,
) -> Result<(), RavenError> {
    let mut fields = Vec::with_capacity(s.fields.len());
    for f in &s.fields {
        let ty = resolve_ty(&f.ty, resolved, env, None)?;
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
    let mut variants = Vec::with_capacity(e.variants.len());
    for v in &e.variants {
        let payload = match &v.payload {
            VariantPayload::Unit => VariantPayloadSig::Unit,
            VariantPayload::Tuple(tys) => {
                let mut out = Vec::with_capacity(tys.len());
                for t in tys {
                    out.push(resolve_ty(t, resolved, env, None)?);
                }
                VariantPayloadSig::Tuple(out)
            }
            VariantPayload::Struct(fields) => {
                let mut out = Vec::with_capacity(fields.len());
                for f in fields {
                    out.push(FieldSig {
                        name: f.name.clone(),
                        ty: resolve_ty(&f.ty, resolved, env, None)?,
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
    let mut methods = HashMap::new();
    for member in &t.members {
        reject_user_generics(
            &member.generics,
            &dummy_decl(member.span.clone()),
            "methods",
        )?;
        let sig = collect_fn_sig(member, resolved, env, None)?;
        methods.insert(member.name.clone(), sig);
    }
    let entry = env.traits.get_mut(&id).expect("trait sig pre populated");
    entry.methods = methods;
    Ok(())
}

fn fill_impl(i: &Impl, resolved: &ResolvedFile<'_>, env: &mut TypeEnv) -> Result<(), RavenError> {
    reject_user_generics(&i.generics, &dummy_decl(i.span.clone()), "impl blocks")?;

    // The implementing type is `for_type` for trait impls; otherwise
    // `trait_or_type` itself.
    let (impl_path, trait_name) = match &i.for_type {
        Some(target) => {
            let trait_name = type_path_name(&i.trait_or_type);
            (target, Some(trait_name))
        }
        None => (&i.trait_or_type, None),
    };
    let self_ty = resolve_type_path(impl_path, resolved, env, None)?;

    let mut methods = HashMap::new();
    for f in &i.items {
        reject_user_generics(&f.generics, &dummy_decl(f.span.clone()), "methods")?;
        let sig = collect_fn_sig(f, resolved, env, Some(&self_ty))?;
        methods.insert(f.name.clone(), sig);
    }
    env.impls.push(ImplSig {
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
) -> Result<FnSig, RavenError> {
    let mut params = Vec::with_capacity(f.params.len());
    let mut has_self = false;
    for p in &f.params {
        if p.name == "self" {
            has_self = true;
            let inner = self_ty.cloned().unwrap_or(Ty::Error);
            params.push(Ty::SelfTy(Box::new(inner)));
        } else {
            params.push(resolve_ty(&p.ty, resolved, env, self_ty)?);
        }
    }
    let ret = match &f.ret {
        Some(t) => resolve_ty(t, resolved, env, self_ty)?,
        None => Ty::Unit,
    };
    Ok(FnSig {
        name: f.name.clone(),
        params,
        ret,
        span: f.span.clone(),
        has_self,
    })
}

/// Resolve an AST `Type` to a `Ty`. `self_ty` is the implementing
/// type when this resolution happens inside an impl block, used to
/// substitute `Self`.
pub fn resolve_ty(
    ty: &Type,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    self_ty: Option<&Ty>,
) -> Result<Ty, RavenError> {
    match &ty.kind {
        TypeKind::Unit => Ok(Ty::Unit),
        TypeKind::Optional(inner) => {
            let t = resolve_ty(inner, resolved, env, self_ty)?;
            Ok(Ty::Option(Box::new(t)))
        }
        TypeKind::Path(p) => resolve_type_path(p, resolved, env, self_ty),
        TypeKind::Dyn(p) => {
            // `dyn Trait` is parsed but not yet supported by the type
            // checker. The resolver has already bound the trait name;
            // we surface a clear error here so users see a hint rather
            // than a panic. The receiving issue #66 will replace this.
            Err(RavenError::ty(
                TypeError::Custom(format!(
                    "`dyn {}` trait objects are not yet supported by the type checker",
                    type_path_name(p)
                )),
                p.span.clone(),
            ))
        }
        TypeKind::Function { params, ret } => {
            let mut ps = Vec::with_capacity(params.len());
            for p in params {
                ps.push(resolve_ty(p, resolved, env, self_ty)?);
            }
            let r = resolve_ty(ret, resolved, env, self_ty)?;
            Ok(Ty::Function {
                params: ps,
                ret: Box::new(r),
            })
        }
    }
}

fn resolve_type_path(
    path: &TypePath,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    self_ty: Option<&Ty>,
) -> Result<Ty, RavenError> {
    let head = &path.segments[0];
    let name = &head.name;

    // Built in primitives and built in generics.
    match name.as_str() {
        "Int" => return ok_zero_generics(head, Ty::Int),
        "Float" => return ok_zero_generics(head, Ty::Float),
        "Bool" => return ok_zero_generics(head, Ty::Bool),
        "Char" => return ok_zero_generics(head, Ty::Char),
        "String" => return ok_zero_generics(head, Ty::Str),
        "Unit" => return ok_zero_generics(head, Ty::Unit),
        "Option" => {
            let inner = expect_one_generic(head, resolved, env, self_ty)?;
            return Ok(Ty::Option(Box::new(inner)));
        }
        "Result" => {
            let (t, e) = expect_two_generics(head, resolved, env, self_ty)?;
            return Ok(Ty::Result(Box::new(t), Box::new(e)));
        }
        "List" | "Array" | "Vec" => {
            let inner = expect_one_generic(head, resolved, env, self_ty)?;
            return Ok(Ty::List(Box::new(inner)));
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
            let s = env
                .structs
                .get(id)
                .ok_or_else(|| RavenError::ty(TypeError::UnknownType(name.clone()), head.span.clone()))?;
            Ok(Ty::Struct {
                id: *id,
                name: s.name.clone(),
            })
        }
        Binding::Enum(id) => {
            let e = env
                .enums
                .get(id)
                .ok_or_else(|| RavenError::ty(TypeError::UnknownType(name.clone()), head.span.clone()))?;
            Ok(Ty::Enum {
                id: *id,
                name: e.name.clone(),
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
    resolve_ty(&seg.generics[0], resolved, env, self_ty)
}

fn expect_two_generics(
    seg: &crate::ast::TypePathSegment,
    resolved: &ResolvedFile<'_>,
    env: &TypeEnv,
    self_ty: Option<&Ty>,
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
    let a = resolve_ty(&seg.generics[0], resolved, env, self_ty)?;
    let b = resolve_ty(&seg.generics[1], resolved, env, self_ty)?;
    Ok((a, b))
}

fn type_path_name(path: &TypePath) -> String {
    path.segments
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn reject_user_generics(
    generics: &[crate::ast::GenericParam],
    decl: &Decl,
    _what: &str,
) -> Result<(), RavenError> {
    if !generics.is_empty() {
        return Err(
            RavenError::ty(TypeError::GenericsNotYetSupported, generics[0].span.clone()).with_hint(
                format!(
                    "user defined generics arrive with issue #59; the item starts at {}",
                    decl.span
                ),
            ),
        );
    }
    Ok(())
}

/// Build a dummy `Decl` carrying a span; used only as input to
/// [`reject_user_generics`] when we already hold a `Function` directly.
fn dummy_decl(span: crate::span::Span) -> Decl {
    use crate::ast::Const;
    use crate::ast::Expr;
    use crate::ast::ExprKind;
    use crate::ast::TypePathSegment;
    let unit_ty = Type {
        kind: TypeKind::Unit,
        span: span.clone(),
    };
    let _ = TypePathSegment {
        name: String::new(),
        generics: vec![],
        span: span.clone(),
    };
    Decl {
        kind: DeclKind::Const(Const {
            name: String::new(),
            ty: unit_ty,
            value: Expr {
                kind: ExprKind::Int(0),
                span: span.clone(),
            },
            span: span.clone(),
        }),
        span,
    }
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
    fn rejects_user_generics() {
        let err = check_source("fun id<T>(x: T) -> T = x\n").unwrap_err();
        match err {
            RavenError::Type(b, _, _) => {
                assert!(matches!(*b, TypeError::GenericsNotYetSupported));
            }
            other => panic!(
                "expected TypeError::GenericsNotYetSupported, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn option_type_path_resolves() {
        // Option in a parameter position; body checking is a later
        // commit so we use a trivial body.
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
