use super::super::{Interpreter, Value};

/// Dispatch methods on `Value::Struct` receivers.
///
/// Thin wrapper around [`Interpreter::call_struct_method`]. When `var_name` is
/// `Some`, mutation of the underlying variable after the call is allowed (used
/// for the `Identifier` receiver path); when `None`, the receiver was an
/// arbitrary expression and no variable write-back happens.
pub(super) fn call(
    interp: &mut Interpreter,
    receiver: Value,
    method_name: &str,
    evaluated_args: Vec<Value>,
    var_name: Option<String>,
) -> Result<Value, String> {
    interp.call_struct_method(receiver, method_name, evaluated_args, var_name)
}
