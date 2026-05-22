use super::super::{Interpreter, Value};

/// Dispatch methods on TCP receivers (`Value::TcpListener` / `Value::TcpStream`).
///
/// At present the language does not expose any methods on these receivers (TCP
/// is driven entirely through builtins like `tcp_listen`, `tcp_accept`, etc.),
/// so this function always reports the method as unknown. Keeping it as its own
/// dispatch arm mirrors the structure of the other receiver kinds and gives a
/// clear home for future TCP methods.
pub(super) fn call(
    _interp: &mut Interpreter,
    receiver: Value,
    method_name: &str,
    _evaluated_args: &[Value],
) -> Result<Value, String> {
    Err(format!(
        "Cannot call method '{}' on value of type {:?}",
        method_name, receiver
    ))
}
