//! Internal type representation.
//!
//! The AST carries a textual `Type` that mirrors what the user wrote.
//! The type checker uses its own [`Ty`] representation. It is resolved
//! (paths are bound to declarations), normalized (the `T?` sugar is
//! lifted into `Option<T>`), and cheap to compare.
//!
//! With generics, `Ty` gains two extra cases:
//!
//! * [`Ty::Param`] is a declared generic parameter, identified by the
//!   declaration that introduces it (its owner span) plus an ordinal
//!   index inside that declaration's parameter list.
//! * [`Ty::Var`] is an inference variable, solved by the union-find
//!   table in [`super::infer::InferCtx`].
//!
//! See `docs/v2/specs/generics.md` for the design rationale.

use crate::resolve::DeclId;
use crate::span::Span;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

/// Identifier for a declared generic parameter.
///
/// `owner` is the introducing declaration's span (matching the resolver's
/// `Binding::GenericParam { owner, name }` value). `index` is the
/// parameter's ordinal position in that declaration's parameter list.
/// The pair is unique across the whole file.
#[derive(Debug, Clone)]
pub struct ParamId {
    pub owner_file: Arc<PathBuf>,
    pub owner_start: usize,
    pub owner_end: usize,
    pub index: usize,
    pub name: String,
}

impl ParamId {
    /// Build a parameter id from an owner span and index.
    pub fn new(owner: &Span, index: usize, name: impl Into<String>) -> Self {
        Self {
            owner_file: owner.file.clone(),
            owner_start: owner.start,
            owner_end: owner.end,
            index,
            name: name.into(),
        }
    }
}

impl PartialEq for ParamId {
    fn eq(&self, other: &Self) -> bool {
        self.owner_start == other.owner_start
            && self.owner_end == other.owner_end
            && self.index == other.index
            && *self.owner_file == *other.owner_file
    }
}

impl Eq for ParamId {}

impl std::hash::Hash for ParamId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.owner_file.hash(state);
        self.owner_start.hash(state);
        self.owner_end.hash(state);
        self.index.hash(state);
    }
}

/// Identifier for an inference variable. Stable across the lifetime of
/// a single [`super::infer::InferCtx`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InferVarId(pub u32);

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
    /// A user declared struct, with its type arguments.
    Struct {
        id: DeclId,
        name: String,
        args: Vec<Ty>,
    },
    /// A user declared enum, with its type arguments.
    Enum {
        id: DeclId,
        name: String,
        args: Vec<Ty>,
    },
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
    /// A declared generic parameter.
    Param(ParamId),
    /// An inference variable.
    Var(InferVarId),
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

    /// True if this type contains any unresolved inference variables.
    pub fn has_var(&self) -> bool {
        match self {
            Ty::Var(_) => true,
            Ty::Option(t) | Ty::List(t) | Ty::SelfTy(t) => t.has_var(),
            Ty::Result(a, b) => a.has_var() || b.has_var(),
            Ty::Struct { args, .. } | Ty::Enum { args, .. } => args.iter().any(|t| t.has_var()),
            Ty::Function { params, ret } => params.iter().any(|t| t.has_var()) || ret.has_var(),
            _ => false,
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
            Ty::Struct { name, args, .. } | Ty::Enum { name, args, .. } => {
                f.write_str(name)?;
                if !args.is_empty() {
                    f.write_str("<")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "{}", a)?;
                    }
                    f.write_str(">")?;
                }
                Ok(())
            }
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
            Ty::Param(p) => f.write_str(&p.name),
            Ty::Var(v) => write!(f, "?{}", v.0),
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
    fn display_struct_with_args() {
        let s = Ty::Struct {
            id: DeclId(0),
            name: "Box".into(),
            args: vec![Ty::Int],
        };
        assert_eq!(format!("{}", s), "Box<Int>");
    }

    #[test]
    fn display_param_and_var() {
        let owner = Span::new(Arc::new(PathBuf::from("t.rv")), 0, 0, 1, 1);
        let p = Ty::Param(ParamId::new(&owner, 0, "T"));
        assert_eq!(format!("{}", p), "T");
        let v = Ty::Var(InferVarId(3));
        assert_eq!(format!("{}", v), "?3");
    }

    #[test]
    fn has_var_walks_recursively() {
        let owner = Span::new(Arc::new(PathBuf::from("t.rv")), 0, 0, 1, 1);
        let _ = ParamId::new(&owner, 0, "T");
        assert!(!Ty::Int.has_var());
        let l = Ty::List(Box::new(Ty::Var(InferVarId(0))));
        assert!(l.has_var());
        let s = Ty::Struct {
            id: DeclId(0),
            name: "Box".into(),
            args: vec![Ty::Var(InferVarId(1))],
        };
        assert!(s.has_var());
    }

    #[test]
    fn error_unifies_marker() {
        assert!(Ty::Error.is_error());
        assert!(!Ty::Int.is_error());
    }
}
