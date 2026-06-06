//! Top level items in HIR.
//!
//! HIR keeps the same kinds of top level items as the AST (function,
//! struct, trait, impl, enum, const, let), but their bodies are
//! lowered. Imports and extern blocks are out of scope for HIR; they
//! stay tied to the resolver output and pass through as a passive list.

use crate::span::Span;
use crate::tycheck::Ty;

use super::expr::{HirBlock, HirExpr};
use super::ty::HirTy;

/// One top level item.
#[derive(Debug, Clone)]
pub struct HirItem {
    pub kind: HirItemKind,
    pub span: Span,
}

/// Top level item kinds.
#[derive(Debug, Clone)]
pub enum HirItemKind {
    Function(HirFn),
    Struct(HirStruct),
    Trait(HirTrait),
    Impl(HirImpl),
    Enum(HirEnum),
    /// `const NAME: T = value`.
    Const {
        name: String,
        ty: HirTy,
        value: HirExpr,
    },
    /// Module-level `let name: T [= init]`.
    Let {
        name: String,
        ty: HirTy,
        init: Option<HirExpr>,
    },
    /// An `extern "ABI" { ... }` block with its resolved foreign
    /// function signatures. The codegen back end declares each as an
    /// imported C-ABI symbol.
    Extern(HirExtern),
    /// Import blocks pass through opaquely. Lowering does not touch them.
    Opaque(String),
}

/// An extern block in HIR, carrying resolved signatures.
#[derive(Debug, Clone)]
pub struct HirExtern {
    /// The ABI string, for example `"C"`.
    pub abi: String,
    pub items: Vec<HirExternFn>,
    pub span: Span,
}

/// One foreign function signature in an extern block. The symbol is the
/// raw C name; the parameter and return types are resolved FFI types the
/// back end maps to C ABI machine types.
#[derive(Debug, Clone)]
pub struct HirExternFn {
    pub name: String,
    pub params: Vec<HirTy>,
    pub ret: HirTy,
    /// True for a variadic C function (`fun printf(fmt: CStr, ...)`).
    pub variadic: bool,
    pub span: Span,
}

/// A function (or method) declaration in HIR.
#[derive(Debug, Clone)]
pub struct HirFn {
    pub name: String,
    pub params: Vec<(String, HirTy, Span)>,
    pub ret: HirTy,
    /// The function's generic parameters in declaration order (impl/trait
    /// owner parameters first, then the function's own). Monomorphization
    /// uses this so a parameter that appears only in the body, for example
    /// the `T` of `fun describe<T>() -> String { type_name<T>() }`, still
    /// drives specialization even though no signature type mentions it.
    pub generics: Vec<crate::tycheck::ty::ParamId>,
    /// `None` for trait members without a default body.
    pub body: Option<HirBlock>,
    pub span: Span,
}

/// A struct declaration. Field types are resolved.
#[derive(Debug, Clone)]
pub struct HirStruct {
    pub name: String,
    pub fields: Vec<(String, HirTy, Span)>,
    /// `@repr(C)`: C memory layout, eligible to cross the FFI by value.
    pub repr_c: bool,
    pub span: Span,
}

/// One enum variant.
#[derive(Debug, Clone)]
pub struct HirVariant {
    pub name: String,
    /// Empty when the variant has no payload. Field names are present
    /// for record style variants; positional variants have synthesized
    /// names (`0`, `1`, ...) so MIR can address them uniformly.
    pub fields: Vec<(String, HirTy, Span)>,
    pub span: Span,
}

/// An enum declaration.
#[derive(Debug, Clone)]
pub struct HirEnum {
    pub name: String,
    pub variants: Vec<HirVariant>,
    pub span: Span,
}

/// A trait declaration with its methods.
#[derive(Debug, Clone)]
pub struct HirTrait {
    pub name: String,
    pub methods: Vec<HirFn>,
    pub span: Span,
}

/// An impl block with its methods.
#[derive(Debug, Clone)]
pub struct HirImpl {
    /// The implementing type's source-level name (for diagnostics).
    pub self_name: String,
    /// The resolved implementing type. The MIR pass mangles it to build
    /// each method's symbol so per-type methods get unique names.
    pub self_ty: Ty,
    /// `Some(trait_name)` for a trait impl, `None` for an inherent impl.
    pub trait_name: Option<String>,
    pub methods: Vec<HirFn>,
    pub span: Span,
}
