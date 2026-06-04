//! Value expressions in HIR.
//!
//! The expression enum is intentionally smaller than the AST's because
//! sugar has been desugared. Notable differences from the AST:
//!
//! * `For`, `Try`, `Range`, and string interpolation are gone in their
//!   surface forms; they appear here as loops, matches, range-new
//!   intrinsics, and explicit interpolation parts.
//! * `If` and `Match` always carry a typed value; the in-statement-only
//!   form does not exist at HIR level.
//! * Single expression function bodies are converted to blocks.

use crate::span::Span;

use super::pattern::HirPattern;
use super::stmt::HirStmt;
use super::ty::HirTy;

/// One expression node, with type and span.
#[derive(Debug, Clone)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty: HirTy,
    pub span: Span,
}

/// One block: a sequence of statements with an optional trailing
/// expression. A block without a trailing expression evaluates to
/// `Unit`. HIR always lowers blocks to this canonical shape.
#[derive(Debug, Clone)]
pub struct HirBlock {
    pub stmts: Vec<HirStmt>,
    pub tail: Option<Box<HirExpr>>,
    pub ty: HirTy,
    pub span: Span,
}

/// Unary prefix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirUnaryOp {
    Neg,
    Not,
    Ref,
}

/// The raw-pointer FFI operations carried by [`HirExprKind::PtrBuiltin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtrBuiltinOp {
    /// `__ptr_alloc<T>(count)`: malloc `count * sizeof(T)` bytes.
    Alloc,
    /// `__ptr_free<T>(p)`: free a buffer from `Alloc`.
    Free,
    /// `__ptr_load<T>(p)`: read the pointee at `p`.
    Load,
    /// `__ptr_store<T>(p, value)`: write `value` at `p`.
    Store,
    /// `__ptr_offset<T>(p, count)`: advance by `count` elements.
    Offset,
    /// `__ptr_is_null<T>(p)`: true when `p` is the null pointer.
    IsNull,
    /// `__ptr_null<T>()`: the null `CPtr<T>`.
    Null,
}

/// The runtime reflection operations carried by
/// [`HirExprKind::ReflectBuiltin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflectBuiltinOp {
    /// `to_any<T>(v)`: box `v` into an `Any` tagged with `T`'s runtime id.
    ToAny,
    /// `cast<T>(a)`: checked downcast of an `Any` to `Option<T>`.
    Cast,
    /// `type_name_of(a)`: the runtime type name of the value in `a`.
    TypeNameOf,
    /// `field_names_of(a)`: the struct field names of the value in `a`.
    FieldNamesOf,
    /// `get_field(a, name)`: the named field of the struct in `a`, boxed.
    GetField,
    /// `set_field(a, name, value)`: write `value` into the named field.
    SetField,
}

/// Binary infix operators. Compound assignment is gone (it is desugared
/// in the lowering pass), so this matches the AST one for one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

/// One arm of a HIR match.
#[derive(Debug, Clone)]
pub struct HirArm {
    pub pattern: HirPattern,
    pub guard: Option<HirExpr>,
    pub body: HirExpr,
    pub span: Span,
}

/// One fragment of a string interpolation literal. The HIR keeps the
/// structured form so MIR / codegen can decide how to concatenate.
#[derive(Debug, Clone)]
pub enum InterpolPart {
    /// A literal text chunk between embedded expressions.
    Text(String),
    /// An embedded expression that should be stringified.
    Expr(HirExpr),
}

/// Expression node kinds.
#[derive(Debug, Clone)]
pub enum HirExprKind {
    // ----- literals -----
    Int(i64),
    Float(f64),
    Bool(bool),
    /// A plain string literal with no embedded `${...}` interpolation.
    Str(String),
    /// A character literal.
    Char(char),
    /// A C string literal (passed verbatim to FFI consumers).
    CStr(String),
    /// The unit value `()`.
    Unit,

    // ----- names -----
    /// A name reference. The resolver's binding is recorded on the
    /// original span; downstream passes can re-look-up if needed.
    Ident(String),
    /// The `self` value inside an impl method.
    SelfValue,

    // ----- aggregates -----
    /// `[a, b, c]` array literal.
    Array(Vec<HirExpr>),
    /// `Name { f1: e1, f2: e2 }` struct literal.
    StructLit {
        name: String,
        fields: Vec<(String, HirExpr)>,
    },
    /// A parenthesized expression, retained so spans cover the parens.
    Paren(Box<HirExpr>),
    /// A block expression `{ stmts; tail? }`.
    Block(HirBlock),

    // ----- operators -----
    Unary {
        op: HirUnaryOp,
        operand: Box<HirExpr>,
    },
    Binary {
        op: HirBinaryOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },

    // ----- calls, fields, indexing -----
    Call {
        callee: Box<HirExpr>,
        args: Vec<HirExpr>,
        /// Resolved explicit type arguments written at the call site
        /// (`f<Int>()`), in declaration order. Carried so MIR can bind a
        /// callee's generic parameters that the value arguments do not pin
        /// down. Empty when the call wrote no type arguments.
        type_args: Vec<super::ty::HirTy>,
    },
    MethodCall {
        receiver: Box<HirExpr>,
        name: String,
        args: Vec<HirExpr>,
    },
    /// `Type.func(args)`: an associated function call with no receiver.
    /// `self_ty` is the implementing type the function is declared on,
    /// used by MIR to build the per-type symbol and (for a generic type)
    /// queue the right instantiation.
    AssocCall {
        self_ty: HirTy,
        name: String,
        args: Vec<HirExpr>,
    },
    Field {
        receiver: Box<HirExpr>,
        name: String,
    },
    Index {
        receiver: Box<HirExpr>,
        index: Box<HirExpr>,
    },

    // ----- control flow -----
    If {
        cond: Box<HirExpr>,
        then_block: HirBlock,
        else_block: Option<HirBlock>,
    },
    Match {
        scrutinee: Box<HirExpr>,
        arms: Vec<HirArm>,
    },
    Loop(HirBlock),
    While {
        cond: Box<HirExpr>,
        body: HirBlock,
    },
    /// `return expr?`. Models a return inside an expression position,
    /// produced by `?` lowering.
    Return(Option<Box<HirExpr>>),
    /// `break expr?`.
    Break(Option<Box<HirExpr>>),
    /// `continue`.
    Continue,

    // ----- desugared sugar -----
    /// A string with interpolated segments, split into `InterpolPart`s.
    Interpolate(Vec<InterpolPart>),
    /// `RangeNew(start, end, inclusive)` built-in. Models the range
    /// constructor produced by lowering `a..b` and `a..=b`.
    RangeNew {
        start: Box<HirExpr>,
        end: Box<HirExpr>,
        inclusive: bool,
    },
    /// `IterNew(source)` built-in. Wraps an iterable value so it can be
    /// driven by `IterNext`. Produced only by `for` lowering.
    IterNew(Box<HirExpr>),
    /// `IterNext(iter)` built-in. Returns `Option<T>` for the next
    /// element of an iterator state.
    IterNext(Box<HirExpr>),
    /// `Ok(expr)` constructor literal, synthesized by `?` lowering on
    /// `Result<T, E>` when the inner expression is fine.
    OkCtor(Box<HirExpr>),
    /// `Err(expr)` constructor literal, synthesized by `?` lowering on
    /// `Result<T, E>`.
    ErrCtor(Box<HirExpr>),
    /// `Some(expr)` constructor literal.
    SomeCtor(Box<HirExpr>),
    /// `None` constructor literal.
    NoneCtor,
    /// A user enum variant construction `EnumName.Variant(args)`. The
    /// result type (the enum, with concrete type arguments) lives on the
    /// `HirExpr.ty`. `variant` is the variant's index in declaration
    /// order, matching what patterns and codegen use.
    EnumCreate {
        variant: usize,
        args: Vec<HirExpr>,
    },

    /// `type_name<T>()` compile-time reflection builtin. Carries the
    /// resolved type argument (a `Ty::Param` for a generic parameter, or a
    /// concrete type), grounded per monomorphization in MIR and rendered to
    /// a `String` constant. See `docs/v2/specs/reflection.md`.
    TypeName(HirTy),
    /// `field_names<T>()` compile-time reflection builtin. Carries the
    /// resolved struct type argument, grounded per monomorphization in MIR
    /// and lowered to a `List<String>` of the struct's field names.
    FieldNames(HirTy),
    /// `field_types<T>()` compile-time reflection builtin. Like
    /// `FieldNames`, but lowered to a `List<String>` of each field's type
    /// name in declaration order, grounded per monomorphization so a generic
    /// field renders its concrete type.
    FieldTypes(HirTy),
    /// `variant_names<T>()` compile-time reflection builtin. Carries the
    /// resolved enum type argument, lowered to a `List<String>` of the
    /// enum's variant names in declaration order.
    VariantNames(HirTy),
    /// `variant_field_types<T>()` compile-time reflection builtin. Lowered
    /// to a `List<List<String>>`: one inner list per variant (declaration
    /// order) of that variant's payload field type names (empty for a unit
    /// variant), grounded per monomorphization.
    VariantFieldTypes(HirTy),

    /// A raw-pointer FFI builtin (`__ptr_load`, `__ptr_store`,
    /// `__ptr_offset`, `__ptr_is_null`, `__ptr_null`, `__ptr_alloc`,
    /// `__ptr_free`). `pointee` is the resolved type argument `T`, grounded
    /// per monomorphization in MIR to pick the load/store width and element
    /// size. `args` are the lowered value operands. See
    /// `docs/v2/specs/std-ffi.md`.
    PtrBuiltin {
        op: PtrBuiltinOp,
        pointee: HirTy,
        args: Vec<HirExpr>,
    },

    /// A runtime reflection builtin (`to_any`, `cast`, `type_name_of`,
    /// `field_names_of`, `get_field`). `type_arg` carries the resolved type
    /// argument `T` for `to_any<T>` (the boxed type) and `cast<T>` (the
    /// downcast target), grounded per monomorphization in MIR so the box or
    /// compare uses the concrete runtime type tag; it is `None` for the
    /// other three. `args` are the lowered value operands. See
    /// `docs/v2/specs/runtime-reflection.md`.
    ReflectBuiltin {
        op: ReflectBuiltinOp,
        type_arg: Option<HirTy>,
        args: Vec<HirExpr>,
    },

    // ----- lambda -----
    Lambda {
        params: Vec<(String, HirTy, Span)>,
        ret: HirTy,
        body: HirBlock,
    },

    /// Unsize a concrete value to a `dyn Trait` value. Synthesized by HIR
    /// lowering at each coercion site the type checker recorded. The
    /// inner expression's type is the concrete type; this node's type is
    /// the `dyn Trait` target.
    DynCoerce {
        value: Box<HirExpr>,
        /// The target trait's short name.
        trait_name: String,
        /// The trait's method names in declaration order (vtable slots).
        methods: Vec<String>,
        /// The concrete source type being coerced.
        concrete_ty: HirTy,
    },
}
