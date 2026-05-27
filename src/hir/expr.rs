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
    },
    MethodCall {
        receiver: Box<HirExpr>,
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

    // ----- lambda -----
    Lambda {
        params: Vec<(String, HirTy, Span)>,
        ret: HirTy,
        body: HirBlock,
    },
}
