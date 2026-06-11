//! Top level declarations.
//!
//! Items appear at the module level: functions, types, traits, impls,
//! enums, extern blocks, imports, constants, and module level lets.

use crate::lexer::Token;
use crate::span::Span;

use super::expr::{Block, Expr};
use super::stmt::Stmt;
use super::ty::{Type, TypePath};

/// A top level declaration with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Decl {
    pub kind: DeclKind,
    pub span: Span,
}

/// Top level item kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum DeclKind {
    /// `fun name<G>(params) -> Ret { body }` or `fun ... = expr`.
    Function(Function),
    /// `struct Name<G> { fields }`.
    Struct(Struct),
    /// `trait Name<G> { members }`.
    Trait(Trait),
    /// `impl<G> Path` or `impl<G> Trait for Type` blocks.
    Impl(Impl),
    /// `enum Name<G> { variants }`.
    Enum(Enum),
    /// `extern "ABI" { signatures }`.
    Extern(Extern),
    /// `import path [as alias] [{ idents }]`.
    Import(Import),
    /// `const NAME: T = expr`.
    Const(Const),
    /// Module level `let name [: T] [= expr]`. Mutable module global.
    Let(LetDecl),
    /// `macro name { (matcher) => { template } ... }`. Macros are expanded by
    /// a token-level pre-pass before the compiler parses, so this node is
    /// produced only by the formatter (which parses un-expanded source); the
    /// compile pipeline never sees it.
    Macro(MacroDef),
}

/// A declarative macro definition, kept as raw tokens for the formatter to
/// render. `body` is every token between the outer braces of
/// `macro name { ... }`.
#[derive(Debug, Clone, PartialEq)]
pub struct MacroDef {
    pub name: String,
    pub body: Vec<Token>,
    pub span: Span,
}

/// A function declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: FunctionBody,
    pub span: Span,
}

/// The body of a function declaration: block or single expression.
#[derive(Debug, Clone, PartialEq)]
pub enum FunctionBody {
    Block(Block),
    Expr(Expr),
    /// A trait member with no default body.
    None,
}

/// One generic parameter declaration: `T: Bound1 + Bound2`.
#[derive(Debug, Clone, PartialEq)]
pub struct GenericParam {
    pub name: String,
    pub bounds: Vec<TypePath>,
    pub span: Span,
}

/// One function parameter: `name: Type`.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

/// A struct declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Struct {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub fields: Vec<StructField>,
    /// Trait names requested by a preceding `@derive(...)` attribute. The
    /// derive pass synthesizes one impl per name before resolution; empty
    /// when no attribute is present.
    pub derives: Vec<String>,
    /// Set by a preceding `@repr(C)` attribute. A repr(C) struct has C
    /// memory layout (fields in declaration order, naturally aligned) and
    /// may cross the FFI boundary by value. See `docs/v2/specs/std-ffi.md`.
    pub repr_c: bool,
    pub span: Span,
}

/// One named field of a struct.
#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

/// A trait declaration, holding zero or more member signatures (with
/// optional default bodies).
#[derive(Debug, Clone, PartialEq)]
pub struct Trait {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub members: Vec<Function>,
    pub span: Span,
}

/// An impl block: either an inherent impl `impl Path { ... }` or a
/// trait impl `impl Trait for Type { ... }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Impl {
    pub generics: Vec<GenericParam>,
    /// For an inherent impl, this is the implementing type's path. For a
    /// trait impl, this is the trait's path.
    pub trait_or_type: TypePath,
    /// For a trait impl, the implementing type. `None` for inherent
    /// impls.
    pub for_type: Option<TypePath>,
    pub items: Vec<Function>,
    pub span: Span,
}

/// An enum declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Enum {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub variants: Vec<EnumVariant>,
    /// Trait names requested by a preceding `@derive(...)` attribute. See
    /// [`Struct::derives`].
    pub derives: Vec<String>,
    pub span: Span,
}

/// One enum variant.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub payload: VariantPayload,
    pub span: Span,
}

/// What payload an enum variant carries.
#[derive(Debug, Clone, PartialEq)]
pub enum VariantPayload {
    /// `Color` (no payload).
    Unit,
    /// `Pair(Int, String)` (positional types).
    Tuple(Vec<Type>),
    /// `Person { name: String, age: Int }` (named fields).
    Struct(Vec<StructField>),
}

/// An extern block: a sequence of foreign function signatures.
#[derive(Debug, Clone, PartialEq)]
pub struct Extern {
    pub abi: String,
    pub items: Vec<ExternFn>,
    pub span: Span,
}

/// One signature inside an extern block.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternFn {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    /// True when the signature ends with `...`: extra C-FFI integer/pointer
    /// arguments may be passed at a call (a variadic function like `printf`).
    pub variadic: bool,
    pub span: Span,
}

/// One name in an import selector list. `import path { a, b as c }` yields
/// selectors `a` (no rename) and `b as c` (bound locally as `c`).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportSelector {
    /// The name as exported by the source module.
    pub name: String,
    /// The local name to bind it under, from `name as local`. `None` binds
    /// under `name` itself.
    pub alias: Option<String>,
}

impl ImportSelector {
    /// The name this selector binds in the importing module: the rename when
    /// present, otherwise the exported name.
    pub fn local(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

/// An import declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub source: ImportSource,
    /// `import path as alias` renames the binding to `alias`.
    pub alias: Option<String>,
    /// `import path { a, b as c }` selects specific names. Empty when no
    /// selector list is present.
    pub selectors: Vec<ImportSelector>,
    pub span: Span,
}

/// The thing being imported.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportSource {
    /// `std/io`, `std/collections/Map`.
    Std(Vec<String>),
    /// A quoted path: `"github.com/user/repo"` or `"./relative"`.
    Quoted(String),
}

/// A `const` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Const {
    pub name: String,
    pub ty: Type,
    pub value: Expr,
    pub span: Span,
}

/// A module level `let` declaration. The initializer is required at
/// parse time at the module level when no type annotation is present
/// and optional otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct LetDecl {
    pub name: String,
    pub ty: Option<Type>,
    pub init: Option<Expr>,
    pub span: Span,
}

/// Convenience: wrap a function body block as a `Stmt::Expr(Block)` for
/// places that need to handle blocks generically. Not used at parse
/// time; left here as a hook for desugaring passes.
#[allow(dead_code)]
fn _block_to_stmts(block: &Block) -> &[Stmt] {
    &block.stmts
}
