//! Type compatibility and unification helpers.
//!
//! This release does not have inference variables, so unification
//! reduces to structural equality with the `Error` placeholder acting
//! as a wildcard so a single typo does not cascade.

use super::ty::Ty;

/// True if `actual` is assignable to `expected`.
///
/// Rules:
///
/// * `Ty::Error` unifies with anything (used to suppress cascading
///   errors after the first one).
/// * `SelfTy(inner)` unifies with `inner` and vice versa.
/// * Otherwise types must be structurally equal.
pub fn assignable(expected: &Ty, actual: &Ty) -> bool {
    if expected.is_error() || actual.is_error() {
        return true;
    }
    let e = expected.strip_self();
    let a = actual.strip_self();
    match (e, a) {
        (Ty::Option(x), Ty::Option(y)) => assignable(x, y),
        (Ty::Result(t1, e1), Ty::Result(t2, e2)) => assignable(t1, t2) && assignable(e1, e2),
        (Ty::List(x), Ty::List(y)) => assignable(x, y),
        (
            Ty::Function {
                params: pa,
                ret: ra,
            },
            Ty::Function {
                params: pb,
                ret: rb,
            },
        ) => {
            pa.len() == pb.len()
                && pa.iter().zip(pb.iter()).all(|(x, y)| assignable(x, y))
                && assignable(ra, rb)
        }
        _ => e == a,
    }
}

/// Unify two branch types (the bodies of an `if` or `match`). If both
/// agree, returns their type; otherwise returns `None`. `Error`
/// branches inherit the other branch's type.
pub fn unify_branches(a: &Ty, b: &Ty) -> Option<Ty> {
    if a.is_error() {
        return Some(b.clone());
    }
    if b.is_error() {
        return Some(a.clone());
    }
    if assignable(a, b) {
        return Some(a.clone());
    }
    if assignable(b, a) {
        return Some(b.clone());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_assignable_only_to_themselves() {
        assert!(assignable(&Ty::Int, &Ty::Int));
        assert!(!assignable(&Ty::Int, &Ty::Float));
        assert!(!assignable(&Ty::Bool, &Ty::Int));
    }

    #[test]
    fn error_unifies_with_anything() {
        assert!(assignable(&Ty::Error, &Ty::Int));
        assert!(assignable(&Ty::Int, &Ty::Error));
    }

    #[test]
    fn option_arms_recurse() {
        assert!(assignable(
            &Ty::Option(Box::new(Ty::Int)),
            &Ty::Option(Box::new(Ty::Int))
        ));
        assert!(!assignable(
            &Ty::Option(Box::new(Ty::Int)),
            &Ty::Option(Box::new(Ty::Bool))
        ));
    }

    #[test]
    fn unify_branches_picks_common_type() {
        assert_eq!(unify_branches(&Ty::Int, &Ty::Int), Some(Ty::Int));
        assert_eq!(unify_branches(&Ty::Error, &Ty::Int), Some(Ty::Int));
        assert_eq!(unify_branches(&Ty::Int, &Ty::Bool), None);
    }
}
