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
    /// Import or extern blocks pass through opaquely. Lowering does not
    /// touch them.
    Opaque(String),
}

/// A function (or method) declaration in HIR.
#[derive(Debug, Clone)]
pub struct HirFn {
    pub name: String,
    pub params: Vec<(String, HirTy, Span)>,
    pub ret: HirTy,
    /// `None` for trait members without a default body.
    pub body: Option<HirBlock>,
    pub span: Span,
}

/// A struct declaration. Field types are resolved.
#[derive(Debug, Clone)]
pub struct HirStruct {
    pub name: String,
    pub fields: Vec<(String, HirTy, Span)>,
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
