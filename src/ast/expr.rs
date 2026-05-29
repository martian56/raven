//! Value expressions.
//!
//! Expressions are the recursive heart of the language. Most of the
//! grammar's nesting lives here: arithmetic, calls, control flow,
//! lambdas, struct literals. The variants below mirror the grammar in
//! `docs/v2/specs/parser.md` closely.

use crate::span::Span;

use super::pattern::Pattern;
use super::stmt::Stmt;
use super::ty::Type;

/// An expression with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

/// All expression node kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    // ----- literals -----
    /// Integer literal.
    Int(i64),
    /// Floating point literal.
    Float(f64),
    /// Boolean literal.
    Bool(bool),
    /// Regular `"..."` string literal with no `${...}` interpolation.
    Str(String),
    /// A string literal containing one or more `${...}` interpolation
    /// segments, split into an ordered run of literal-text and embedded
    /// expression fragments. The parser produces this only when at least
    /// one real (non-escaped) `${...}` is present; otherwise the literal
    /// stays a plain `Str`. See `docs/v2/specs/interpolation.md`.
    InterpolatedString(Vec<StrFragment>),
    /// Triple quoted block string literal. Block strings are raw and are
    /// never interpolated.
    BlockStr(String),
    /// Character literal.
    Char(char),
    /// `c"..."` FFI string.
    CStr(String),
    /// `self` keyword.
    SelfLower,
    /// `Self` keyword (the enclosing type).
    SelfUpper,

    // ----- names and aggregates -----
    /// A bare identifier, optionally with generic arguments
    /// `name<T1, T2>`.
    Ident { name: String, generics: Vec<Type> },
    /// A struct literal: `Point { x: 1, y: 2 }`. The `name` is the type
    /// path source spelling, recorded as identifier segments. Generic
    /// arguments on the path live on the leading `Ident` if any.
    StructLit {
        name: String,
        generics: Vec<Type>,
        fields: Vec<FieldInit>,
    },
    /// `[a, b, c]` array literal.
    Array(Vec<Expr>),
    /// `{a, b, c}` set literal. Always at least one element (an empty set
    /// is written `Set.new()` and a single-element `{x}` is a block). The
    /// HIR lowers this to a `Set.new()` constructor plus one `add` call
    /// per element.
    SetLit(Vec<Expr>),
    /// `[k1: v1, k2: v2]` map literal. The empty map is the distinct `[:]`
    /// form, which still lowers here with no pairs. The HIR lowers this to
    /// a `Map.new()` constructor plus one `set` call per pair.
    MapLit(Vec<(Expr, Expr)>),
    /// `(a, b, c)` tuple literal. Always at least two elements. The
    /// parser produces this and the resolver rejects it until tuples
    /// land; see `docs/v2/specs/parser.md`.
    Tuple(Vec<Expr>),
    /// `(expr)` parenthesized expression, retained so spans cover the
    /// parens for error reporting.
    Paren(Box<Expr>),
    /// `{ stmts...; trailing? }` block expression.
    Block(Block),

    // ----- operators -----
    /// Unary prefix operator.
    Unary { op: UnaryOp, operand: Box<Expr> },
    /// Binary infix operator.
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// Range expression: `start..end` or `start..=end`.
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
    },

    // ----- calls, fields, indexing -----
    /// Function call: `callee(arg1, arg2)`.
    Call { callee: Box<Expr>, args: Vec<Expr> },
    /// Method call: `receiver.name<G>(arg1, ...)`.
    MethodCall {
        receiver: Box<Expr>,
        name: String,
        generics: Vec<Type>,
        args: Vec<Expr>,
    },
    /// Field access: `receiver.name`. No call parens.
    Field { receiver: Box<Expr>, name: String },
    /// Indexing: `receiver[index]`.
    Index {
        receiver: Box<Expr>,
        index: Box<Expr>,
    },
    /// `expr?` Result/Option propagation.
    Try(Box<Expr>),

    // ----- control flow -----
    /// `if cond { ... } else if cond { ... } else { ... }`.
    If {
        cond: Box<Expr>,
        then_branch: Block,
        /// Either another `If` expression or a final `Block`.
        else_branch: Option<Box<ElseBranch>>,
    },
    /// `match scrutinee { arms... }`.
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    /// `loop { ... }`.
    Loop(Block),
    /// `while cond { ... }`.
    While { cond: Box<Expr>, body: Block },
    /// `for pat in iter { ... }`.
    For {
        pattern: Pattern,
        iter: Box<Expr>,
        body: Block,
    },
    /// Anonymous function expression. `params_inferred` is true when the
    /// parser used the shorthand `{ x, y -> body }` form (no annotated
    /// parameter types, no return type). The full `fun(...) -> T { }`
    /// form sets it to false.
    Lambda {
        params: Vec<LambdaParam>,
        ret: Option<Type>,
        body: LambdaBody,
        params_inferred: bool,
    },
}

/// One fragment of an interpolated string literal. An interpolated
/// string is an ordered sequence of these: literal text chunks and
/// embedded expressions, in source order.
#[derive(Debug, Clone, PartialEq)]
pub enum StrFragment {
    /// A run of literal characters between embedded expressions. Already
    /// escape-decoded by the lexer (and with any `\$` un-escaped).
    Literal(String),
    /// An embedded `${expr}` expression, parsed as a normal Raven
    /// expression.
    Expr(Box<Expr>),
}

/// Unary prefix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `-x` arithmetic negation.
    Neg,
    /// `!x` logical not.
    Not,
    /// `&x` reference. Semantics deferred to the type checker.
    Ref,
}

/// Binary infix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
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

/// One initializer in a struct literal. Shorthand form `{ name }` is
/// represented by setting `value` to a same span `Ident` with the same
/// name (so downstream passes treat the two forms identically).
#[derive(Debug, Clone, PartialEq)]
pub struct FieldInit {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

/// A block expression: a sequence of statements with an optional trailing
/// expression. When `trailing` is `Some`, the block evaluates to that
/// expression; otherwise it evaluates to `()`.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub trailing: Option<Box<Expr>>,
    pub span: Span,
}

/// One arm in a `match` expression: `pat if guard -> body`.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

/// Either an `else if ...` (recursive) or a final `else { ... }` block.
#[derive(Debug, Clone, PartialEq)]
pub enum ElseBranch {
    If(Expr),
    Block(Block),
}

/// One parameter in a lambda. For shorthand `{ x, y -> body }` lambdas
/// the `ty` is `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct LambdaParam {
    pub name: String,
    pub ty: Option<Type>,
    pub span: Span,
}

/// A lambda body is either a block or a single expression after `=`.
#[derive(Debug, Clone, PartialEq)]
pub enum LambdaBody {
    Block(Block),
    Expr(Box<Expr>),
}
