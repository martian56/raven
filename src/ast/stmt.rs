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
    /// `let name: T = expr`, `let name = expr`, or `const name: T = expr`.
    /// `mutable` is `true` for `let` and `false` for `const`; a `const`
    /// local is an immutable binding the type checker forbids reassigning.
    /// The `init` is `None` only at top level where `let name: T` declares
    /// an uninitialized module global. Inside a function body the parser
    /// rejects missing initializers.
    Let {
        name: String,
        ty: Option<Type>,
        init: Option<Expr>,
        mutable: bool,
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
    /// `let (a, b) = e`; desugared after parsing.
    LetTuple {
        pattern: super::pattern::Pattern,
        init: Expr,
        mutable: bool,
    },
    /// `assert cond` or `assert cond, message`; desugared after parsing.
    /// `text` is the condition as written, for the failure message.
    Assert {
        cond: Expr,
        message: Option<Expr>,
        text: String,
    },
    /// `let P = e else { .. }`; desugared after parsing.
    LetElse {
        pattern: super::pattern::Pattern,
        init: Expr,
        else_block: super::expr::Block,
        mutable: bool,
    },
}

/// `target[index] = value` on a class, as the call `target.set_index(index,
/// value)`. A compound `op=` passes `target[index] op value`.
pub fn set_index_call(
    target: &Expr,
    op: AssignOp,
    value: &Expr,
    span: &crate::span::Span,
) -> Option<Expr> {
    use super::expr::ExprKind;
    let ExprKind::Index { receiver, index } = &target.kind else {
        return None;
    };
    let value = match op.binary() {
        None => value.clone(),
        Some(bin) => Expr {
            kind: ExprKind::Binary {
                op: bin,
                lhs: Box::new(target.clone()),
                rhs: Box::new(value.clone()),
            },
            span: span.in_copy(crate::span::COMPOUND_INDEX_COPY),
        },
    };
    Some(Expr {
        kind: ExprKind::MethodCall {
            receiver: receiver.clone(),
            name: "set_index".into(),
            generics: Vec::new(),
            args: vec![(**index).clone(), value],
        },
        span: span.clone(),
    })
}

impl AssignOp {
    /// The binary operator a compound assignment applies, `None` for `=`.
    pub fn binary(self) -> Option<super::expr::BinaryOp> {
        use super::expr::BinaryOp;
        Some(match self {
            AssignOp::Assign => return None,
            AssignOp::Add => BinaryOp::Add,
            AssignOp::Sub => BinaryOp::Sub,
            AssignOp::Mul => BinaryOp::Mul,
            AssignOp::Div => BinaryOp::Div,
            AssignOp::Mod => BinaryOp::Mod,
            AssignOp::BitAnd => BinaryOp::BitAnd,
            AssignOp::BitOr => BinaryOp::BitOr,
            AssignOp::BitXor => BinaryOp::BitXor,
            AssignOp::Shl => BinaryOp::Shl,
            AssignOp::Shr => BinaryOp::Shr,
        })
    }
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
