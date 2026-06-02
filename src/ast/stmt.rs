//! Statements.
//!
//! Statements appear inside block expressions and as top level module
//! items (for `let` and `const`, which double as item declarations).
//! Assignment is a statement here, not an expression: see the spec for
//! the disambiguation rule.

use crate::span::Span;

use super::expr::Expr;
use super::ty::Type;

/// A statement with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

/// Statement node kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// `let name: T = expr` or `let name = expr`. The `init` is `None`
    /// only at top level where `let name: T` declares an uninitialized
    /// module global. Inside a function body the parser rejects missing
    /// initializers.
    Let {
        name: String,
        ty: Option<Type>,
        init: Option<Expr>,
    },
    /// `return expr?`.
    Return(Option<Expr>),
    /// `break expr?` (carries a value when inside a `loop`).
    Break(Option<Expr>),
    /// `continue`.
    Continue,
    /// `defer expr`: schedules `expr` to run at scope exit.
    Defer(Expr),
    /// `spawn expr`: starts a goroutine running the `fun() -> Unit`
    /// closure `expr` produces.
    Spawn(Expr),
    /// `lvalue op= rhs`. The `target` expression is constrained at parse
    /// time to be a valid LValue (identifier with `.` and `[]` chains).
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
    },
    /// A bare expression evaluated for its side effects (or returned as
    /// the trailing value of a block).
    Expr(Expr),
}

/// Compound assignment operators. The plain `=` form is also represented
/// here as `Assign`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    /// `=`
    Assign,
    /// `+=`
    Add,
    /// `-=`
    Sub,
    /// `*=`
    Mul,
    /// `/=`
    Div,
    /// `%=`
    Mod,
    /// `&=`
    BitAnd,
    /// `|=`
    BitOr,
    /// `^=`
    BitXor,
    /// `<<=`
    Shl,
    /// `>>=`
    Shr,
}
