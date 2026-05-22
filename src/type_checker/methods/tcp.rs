use super::super::{Type, TypeChecker};
use crate::ast::Expression;

/// Type-check method calls whose receiver is a TCP type (`Type::TcpListener` or
/// `Type::TcpStream`).
///
/// At present the language does not expose any methods on these receivers (TCP
/// is driven entirely through builtins like `tcp_listen`, `tcp_accept`, etc.),
/// so this function always reports the method as unknown. Keeping it as its own
/// dispatch arm mirrors the structure of the other receiver kinds and gives a
/// clear home for future TCP methods. The error message matches the
/// general "cannot call method" fallback to preserve user-visible behavior.
pub(super) fn check(
    _tc: &mut TypeChecker,
    receiver_type: Type,
    _method_name: &str,
    _args: &[Expression],
) -> Result<Type, String> {
    Err(format!(
        "Cannot call method on value of type '{}'\n   = help: Methods work on arrays, strings, modules, and structs with impl blocks.",
        receiver_type.fmt_for_user()
    ))
}
