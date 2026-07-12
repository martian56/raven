//! Built in generic types and their inherent methods.
//!
//! `Option<T>`, `Result<T, E>`, and `List<T>` are typed without the
//! full generic mechanism. Each one has a fixed shape and a small set
//! of methods recognized by the method dispatcher.

use super::ty::Ty;

/// What a built in method needs the dispatcher to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinMethod {
    pub name: &'static str,
    /// Parameter types after `self`. Use placeholder types `Element`
    /// and `Error` for `T` and `E`; the dispatcher substitutes them
    /// against the receiver's instantiation.
    pub params: Vec<MethodSlot>,
    pub ret: MethodSlot,
}

/// One slot in a built in method's signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MethodSlot {
    /// A concrete type, no substitution.
    Concrete(Ty),
    /// The receiver's first type parameter (`T` for `Option`, `Result`,
    /// `List`).
    Element,
    /// The receiver's second type parameter (`E` for `Result`).
    Error,
}

impl MethodSlot {
    /// Substitute against the receiver's type. `element` is the `T`
    /// parameter, `error` is the `E` parameter (or `Ty::Error` when
    /// not applicable).
    pub fn substitute(&self, element: &Ty, error: &Ty) -> Ty {
        match self {
            MethodSlot::Concrete(t) => t.clone(),
            MethodSlot::Element => element.clone(),
            MethodSlot::Error => error.clone(),
        }
    }
}

/// Inherent methods on `Option<T>`.
pub fn option_methods() -> Vec<BuiltinMethod> {
    vec![
        BuiltinMethod {
            name: "is_some",
            params: vec![],
            ret: MethodSlot::Concrete(Ty::Bool),
        },
        BuiltinMethod {
            name: "is_none",
            params: vec![],
            ret: MethodSlot::Concrete(Ty::Bool),
        },
        BuiltinMethod {
            name: "unwrap",
            params: vec![],
            ret: MethodSlot::Element,
        },
        BuiltinMethod {
            name: "unwrap_or",
            params: vec![MethodSlot::Element],
            ret: MethodSlot::Element,
        },
    ]
}

/// Inherent methods on `Result<T, E>`.
pub fn result_methods() -> Vec<BuiltinMethod> {
    vec![
        BuiltinMethod {
            name: "is_ok",
            params: vec![],
            ret: MethodSlot::Concrete(Ty::Bool),
        },
        BuiltinMethod {
            name: "is_err",
            params: vec![],
            ret: MethodSlot::Concrete(Ty::Bool),
        },
        BuiltinMethod {
            name: "unwrap",
            params: vec![],
            ret: MethodSlot::Element,
        },
        BuiltinMethod {
            name: "unwrap_or",
            params: vec![MethodSlot::Element],
            ret: MethodSlot::Element,
        },
    ]
}

/// Inherent methods on `List<T>`.
pub fn list_methods() -> Vec<BuiltinMethod> {
    vec![
        BuiltinMethod {
            name: "len",
            params: vec![],
            ret: MethodSlot::Concrete(Ty::Int),
        },
        BuiltinMethod {
            name: "is_empty",
            params: vec![],
            ret: MethodSlot::Concrete(Ty::Bool),
        },
        BuiltinMethod {
            name: "push",
            params: vec![MethodSlot::Element],
            ret: MethodSlot::Concrete(Ty::Unit),
        },
        BuiltinMethod {
            name: "pop",
            params: vec![],
            ret: MethodSlot::Element,
        },
        BuiltinMethod {
            name: "get",
            params: vec![MethodSlot::Concrete(Ty::Int)],
            ret: MethodSlot::Element,
        },
    ]
}

/// Methods on the primitive `String` type.
pub fn string_methods() -> Vec<BuiltinMethod> {
    vec![
        BuiltinMethod {
            name: "len",
            params: vec![],
            ret: MethodSlot::Concrete(Ty::Int),
        },
        BuiltinMethod {
            name: "is_empty",
            params: vec![],
            ret: MethodSlot::Concrete(Ty::Bool),
        },
    ]
}

/// Methods on the primitive `Int` type.
pub fn int_methods() -> Vec<BuiltinMethod> {
    vec![BuiltinMethod {
        name: "to_float",
        params: vec![],
        ret: MethodSlot::Concrete(Ty::Float),
    }]
}

/// Methods on the primitive `Float` type.
pub fn float_methods() -> Vec<BuiltinMethod> {
    vec![BuiltinMethod {
        name: "to_int",
        params: vec![],
        ret: MethodSlot::Concrete(Ty::Int),
    }]
}

/// Look up a method on a built in type. Returns the substituted
/// parameter list and return type when found.
pub fn lookup_method(receiver: &Ty, name: &str) -> Option<(Vec<Ty>, Ty)> {
    let (table, element, error) = match receiver {
        Ty::Option(t) => (option_methods(), t.as_ref().clone(), Ty::Error),
        Ty::Result(t, e) => (result_methods(), t.as_ref().clone(), e.as_ref().clone()),
        Ty::List(t) => (list_methods(), t.as_ref().clone(), Ty::Error),
        Ty::Str => (string_methods(), Ty::Error, Ty::Error),
        Ty::Int => (int_methods(), Ty::Error, Ty::Error),
        Ty::Float => (float_methods(), Ty::Error, Ty::Error),
        _ => return None,
    };
    table.into_iter().find(|m| m.name == name).map(|m| {
        let params = m
            .params
            .iter()
            .map(|p| p.substitute(&element, &error))
            .collect();
        let ret = m.ret.substitute(&element, &error);
        (params, ret)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_unwrap_returns_element() {
        let opt_int = Ty::Option(Box::new(Ty::Int));
        let (params, ret) = lookup_method(&opt_int, "unwrap").expect("unwrap exists");
        assert!(params.is_empty());
        assert_eq!(ret, Ty::Int);
    }

    #[test]
    fn result_unwrap_or_takes_element() {
        let res = Ty::Result(Box::new(Ty::Bool), Box::new(Ty::Str));
        let (params, ret) = lookup_method(&res, "unwrap_or").expect("unwrap_or exists");
        assert_eq!(params, vec![Ty::Bool]);
        assert_eq!(ret, Ty::Bool);
    }

    #[test]
    fn list_push_takes_element() {
        let l = Ty::List(Box::new(Ty::Int));
        let (params, ret) = lookup_method(&l, "push").expect("push exists");
        assert_eq!(params, vec![Ty::Int]);
        assert_eq!(ret, Ty::Unit);
    }

    #[test]
    fn unknown_method_returns_none() {
        let opt = Ty::Option(Box::new(Ty::Int));
        assert!(lookup_method(&opt, "no_such_method").is_none());
    }

    #[test]
    fn numeric_conversions() {
        let (p, r) = lookup_method(&Ty::Int, "to_float").expect("Int has to_float");
        assert!(p.is_empty());
        assert_eq!(r, Ty::Float);
        let (p2, r2) = lookup_method(&Ty::Float, "to_int").expect("Float has to_int");
        assert!(p2.is_empty());
        assert_eq!(r2, Ty::Int);
        // The conversions are not cross available.
        assert!(lookup_method(&Ty::Int, "to_int").is_none());
        assert!(lookup_method(&Ty::Float, "to_float").is_none());
    }
}
