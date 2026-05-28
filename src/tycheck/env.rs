//! Declaration signature environment.
//!
//! After the declared type collection pass, every named item in the
//! file has a signature recorded in [`TypeEnv`]. The body checking pass
//! looks up signatures here when it encounters calls, field access,
//! method dispatch, and enum variant construction.

use std::collections::HashMap;

use crate::resolve::DeclId;
use crate::span::Span;

use super::ty::{ParamId, Ty};

/// One declared generic parameter on a signature, with its trait bounds.
#[derive(Debug, Clone)]
pub struct GenericParamSig {
    /// The parameter id (owner span plus index plus original name).
    pub id: ParamId,
    /// Names of the traits this parameter is constrained by.
    pub bounds: Vec<String>,
    pub span: Span,
}

/// A struct's declared shape: ordered field list and a name to index map.
#[derive(Debug, Clone)]
pub struct StructSig {
    pub name: String,
    pub generics: Vec<GenericParamSig>,
    pub fields: Vec<FieldSig>,
    pub span: Span,
}

impl StructSig {
    /// Look up a field by name; returns `(index, type)` on success.
    pub fn field(&self, name: &str) -> Option<(usize, &Ty)> {
        self.fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == name)
            .map(|(i, f)| (i, &f.ty))
    }
}

/// One field of a struct.
#[derive(Debug, Clone)]
pub struct FieldSig {
    pub name: String,
    pub ty: Ty,
    pub span: Span,
}

/// One enum and its variants.
#[derive(Debug, Clone)]
pub struct EnumSig {
    pub name: String,
    pub generics: Vec<GenericParamSig>,
    pub variants: Vec<VariantSig>,
    pub span: Span,
}

impl EnumSig {
    /// Look up a variant by name; returns `(index, signature)`.
    pub fn variant(&self, name: &str) -> Option<(usize, &VariantSig)> {
        self.variants
            .iter()
            .enumerate()
            .find(|(_, v)| v.name == name)
    }
}

/// One variant of an enum: optional payload.
#[derive(Debug, Clone)]
pub struct VariantSig {
    pub name: String,
    pub payload: VariantPayloadSig,
    pub span: Span,
}

/// What a variant carries.
#[derive(Debug, Clone)]
pub enum VariantPayloadSig {
    /// `Color` (no payload).
    Unit,
    /// `Pair(Int, String)` (positional types).
    Tuple(Vec<Ty>),
    /// `Person { name: String, age: Int }` (named fields).
    Struct(Vec<FieldSig>),
}

/// A function or method signature.
#[derive(Debug, Clone)]
pub struct FnSig {
    pub name: String,
    pub generics: Vec<GenericParamSig>,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub span: Span,
    /// True when this signature came from a method declaration with a
    /// `self` receiver; the parser already includes `self` in the
    /// parameter list, so this flag is purely for diagnostics.
    pub has_self: bool,
}

/// A trait declaration: a list of method signatures keyed by name.
#[derive(Debug, Clone)]
pub struct TraitSig {
    pub name: String,
    pub generics: Vec<GenericParamSig>,
    pub methods: HashMap<String, FnSig>,
    /// Method names in declaration order. The vtable slot order for
    /// `dyn Trait` dispatch follows this list, so the slot index of a
    /// method is its position here.
    pub method_order: Vec<String>,
    pub span: Span,
}

/// One impl block: which type it implements, optionally which trait,
/// and the methods it provides.
#[derive(Debug, Clone)]
pub struct ImplSig {
    /// Generic parameters declared on the impl block itself.
    pub generics: Vec<GenericParamSig>,
    /// The implementing type. `Self` inside the block refers to this.
    pub self_ty: Ty,
    /// The trait this impl satisfies, if any. `None` for inherent impls.
    pub trait_name: Option<String>,
    /// Methods provided by the impl, keyed by name.
    pub methods: HashMap<String, FnSig>,
    pub span: Span,
}

/// The whole environment built by the declared type collection pass.
#[derive(Debug, Clone, Default)]
pub struct TypeEnv {
    /// Function declarations keyed by `DeclId`.
    pub functions: HashMap<DeclId, FnSig>,
    /// Struct declarations keyed by `DeclId`.
    pub structs: HashMap<DeclId, StructSig>,
    /// Enum declarations keyed by `DeclId`.
    pub enums: HashMap<DeclId, EnumSig>,
    /// Trait declarations keyed by `DeclId`.
    pub traits: HashMap<DeclId, TraitSig>,
    /// Impl blocks in declaration order.
    pub impls: Vec<ImplSig>,
    /// Top level const declarations keyed by `DeclId`.
    pub consts: HashMap<DeclId, Ty>,
    /// Top level `let` declarations keyed by `DeclId`.
    pub statics: HashMap<DeclId, Ty>,
    /// Extern declarations keyed by `(DeclId, item_index)`.
    pub externs: HashMap<(DeclId, usize), FnSig>,
}

impl TypeEnv {
    /// Construct an empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up every inherent (no trait) impl method matching `name` on
    /// `self_ty`. The vector is sorted by impl order for stable output.
    pub fn inherent_methods<'a>(&'a self, self_ty: &Ty, name: &str) -> Vec<&'a FnSig> {
        self.impls
            .iter()
            .filter(|i| i.trait_name.is_none() && tys_equal(&i.self_ty, self_ty))
            .filter_map(|i| i.methods.get(name))
            .collect()
    }

    /// Look up every trait impl method matching `name` on `self_ty`.
    /// Returns `(trait_name, signature)` pairs.
    pub fn trait_methods<'a>(&'a self, self_ty: &Ty, name: &str) -> Vec<(&'a str, &'a FnSig)> {
        self.impls
            .iter()
            .filter(|i| i.trait_name.is_some() && tys_equal(&i.self_ty, self_ty))
            .filter_map(|i| {
                i.methods
                    .get(name)
                    .map(|m| (i.trait_name.as_deref().unwrap(), m))
            })
            .collect()
    }
}

/// Type equality used for impl matching. The leading `SelfTy` wrapper
/// is stripped first so an `impl` written against a struct matches a
/// receiver of `SelfTy(Struct)` inside another method body.
pub fn tys_equal(a: &Ty, b: &Ty) -> bool {
    a.strip_self() == b.strip_self()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::DeclId;
    use crate::span::Span;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn span() -> Span {
        Span::new(Arc::new(PathBuf::from("t.rv")), 0, 0, 1, 1)
    }

    #[test]
    fn struct_field_lookup_returns_index() {
        let sig = StructSig {
            name: "Point".into(),
            generics: Vec::new(),
            fields: vec![
                FieldSig {
                    name: "x".into(),
                    ty: Ty::Int,
                    span: span(),
                },
                FieldSig {
                    name: "y".into(),
                    ty: Ty::Int,
                    span: span(),
                },
            ],
            span: span(),
        };
        let (idx, ty) = sig.field("y").expect("y is present");
        assert_eq!(idx, 1);
        assert_eq!(ty, &Ty::Int);
        assert!(sig.field("z").is_none());
    }

    #[test]
    fn enum_variant_lookup() {
        let sig = EnumSig {
            name: "Color".into(),
            generics: Vec::new(),
            variants: vec![
                VariantSig {
                    name: "Red".into(),
                    payload: VariantPayloadSig::Unit,
                    span: span(),
                },
                VariantSig {
                    name: "Rgb".into(),
                    payload: VariantPayloadSig::Tuple(vec![Ty::Int, Ty::Int, Ty::Int]),
                    span: span(),
                },
            ],
            span: span(),
        };
        let (idx, v) = sig.variant("Rgb").expect("Rgb is present");
        assert_eq!(idx, 1);
        assert!(matches!(v.payload, VariantPayloadSig::Tuple(_)));
    }

    #[test]
    fn inherent_methods_match_self_ty() {
        let mut env = TypeEnv::new();
        let mut methods = HashMap::new();
        methods.insert(
            "get".into(),
            FnSig {
                name: "get".into(),
                generics: Vec::new(),
                params: vec![Ty::Int],
                ret: Ty::Int,
                span: span(),
                has_self: true,
            },
        );
        env.impls.push(ImplSig {
            generics: Vec::new(),
            self_ty: Ty::Struct {
                id: DeclId(0),
                name: "Point".into(),
                args: Vec::new(),
            },
            trait_name: None,
            methods,
            span: span(),
        });
        let found = env.inherent_methods(
            &Ty::Struct {
                id: DeclId(0),
                name: "Point".into(),
                args: Vec::new(),
            },
            "get",
        );
        assert_eq!(found.len(), 1);
    }
}
