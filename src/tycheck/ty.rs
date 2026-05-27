//! Internal type representation.
//!
//! The AST carries a textual `Type` that mirrors what the user wrote.
//! The type checker uses its own [`Ty`] representation. It is resolved
//! (paths are bound to declarations), normalized (the `T?` sugar is
//! lifted into `Option<T>`), and cheap to compare.
//!
//! See `docs/v2/specs/tycheck.md` for the design rationale.

use crate::resolve::DeclId;
use std::fmt;

/// A resolved internal type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    /// `()` unit type.
    Unit,
    /// `Bool`.
    Bool,
    /// 64 bit signed integer.
    Int,
    /// 64 bit float.
    Float,
    /// A single Unicode scalar value.
    Char,
    /// A heap allocated string.
    Str,
    /// A user declared struct.
    Struct { id: DeclId, name: String },
    /// A user declared enum.
    Enum { id: DeclId, name: String },
    /// Built in `Option<T>`.
    Option(Box<Ty>),
    /// Built in `Result<T, E>`.
    Result(Box<Ty>, Box<Ty>),
    /// Built in `List<T>` (the type of array literals).
    List(Box<Ty>),
    /// A function value type: `fun(A, B) -> C`.
    Function { params: Vec<Ty>, ret: Box<Ty> },
    /// `Self` inside an `impl` block, bound to the implementing type.
    /// The contained type is the implementing type for convenience.
    SelfTy(Box<Ty>),
    /// A placeholder used when an upstream error already reported the
    /// problem. Always unifies with anything.
    Error,
}

impl Ty {
    /// True if this type is the special `Error` placeholder.
    pub fn is_error(&self) -> bool {
        matches!(self, Ty::Error)
    }

    /// Strip a leading `SelfTy` wrapper. Methods on `impl` blocks see
    /// `self: SelfTy(T)` but inside the body comparisons should treat
    /// the receiver as `T` directly.
    pub fn strip_self(&self) -> &Ty {
        match self {
            Ty::SelfTy(inner) => inner,
            other => other,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Unit => f.write_str("()"),
            Ty::Bool => f.write_str("Bool"),
            Ty::Int => f.write_str("Int"),
            Ty::Float => f.write_str("Float"),
            Ty::Char => f.write_str("Char"),
            Ty::Str => f.write_str("String"),
            Ty::Struct { name, .. } => f.write_str(name),
            Ty::Enum { name, .. } => f.write_str(name),
            Ty::Option(inner) => write!(f, "Option<{}>", inner),
            Ty::Result(t, e) => write!(f, "Result<{}, {}>", t, e),
            Ty::List(inner) => write!(f, "List<{}>", inner),
            Ty::Function { params, ret } => {
                f.write_str("fun(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            Ty::SelfTy(inner) => write!(f, "Self/* = {} */", inner),
            Ty::Error => f.write_str("<error>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_primitive_types() {
        assert_eq!(format!("{}", Ty::Int), "Int");
        assert_eq!(format!("{}", Ty::Bool), "Bool");
        assert_eq!(format!("{}", Ty::Unit), "()");
    }

    #[test]
    fn display_built_in_generics() {
        let opt_int = Ty::Option(Box::new(Ty::Int));
        assert_eq!(format!("{}", opt_int), "Option<Int>");
        let res = Ty::Result(Box::new(Ty::Int), Box::new(Ty::Str));
        assert_eq!(format!("{}", res), "Result<Int, String>");
        let list = Ty::List(Box::new(Ty::Bool));
        assert_eq!(format!("{}", list), "List<Bool>");
    }

    #[test]
    fn display_function_type() {
        let fty = Ty::Function {
            params: vec![Ty::Int, Ty::Int],
            ret: Box::new(Ty::Bool),
        };
        assert_eq!(format!("{}", fty), "fun(Int, Int) -> Bool");
    }

    #[test]
    fn error_unifies_marker() {
        assert!(Ty::Error.is_error());
        assert!(!Ty::Int.is_error());
    }
}
