use super::super::{Interpreter, Value};

/// Dispatch array methods.
///
/// If `var_name` is `Some(name)`, the receiver was an `Identifier` and `push`/`pop`
/// mutate `interp.variables[name]` (and `pop` returns the popped value). If `None`,
/// the receiver was an arbitrary expression: `push` returns a new array without
/// mutating, and `pop` returns the last element without removing it (peek semantics
/// preserved from the original `MethodCall` arm).
pub(super) fn call(
    interp: &mut Interpreter,
    mut elements: Vec<Value>,
    method_name: &str,
    evaluated_args: &[Value],
    var_name: Option<&str>,
) -> Result<Value, String> {
    match method_name {
        "push" => {
            if evaluated_args.len() != 1 {
                return Err(format!(
                    "push() expects 1 argument, got {}",
                    evaluated_args.len()
                ));
            }
            if let Some(name) = var_name {
                elements.push(evaluated_args[0].clone());
                interp
                    .variables
                    .insert(name.to_string(), Value::Array(elements.clone()));
                Ok(Value::Array(elements))
            } else {
                let mut new_elements = elements.clone();
                new_elements.push(evaluated_args[0].clone());
                Ok(Value::Array(new_elements))
            }
        }
        "pop" => {
            if !evaluated_args.is_empty() {
                return Err(format!(
                    "pop() expects 0 arguments, got {}",
                    evaluated_args.len()
                ));
            }
            if elements.is_empty() {
                return Err("Cannot pop from empty array".to_string());
            }
            if let Some(name) = var_name {
                let popped = elements.pop().unwrap();
                interp
                    .variables
                    .insert(name.to_string(), Value::Array(elements));
                Ok(popped)
            } else {
                Ok(elements.last().unwrap().clone())
            }
        }
        "slice" => {
            if evaluated_args.len() != 2 {
                return Err(format!(
                    "slice() expects 2 arguments, got {}",
                    evaluated_args.len()
                ));
            }
            let start = match &evaluated_args[0] {
                Value::Int(i) => *i,
                _ => return Err("slice() start index must be integer".to_string()),
            };
            let end = match &evaluated_args[1] {
                Value::Int(i) => *i,
                _ => return Err("slice() end index must be integer".to_string()),
            };

            if start < 0 || end < 0 || start > end || start as usize >= elements.len() {
                return Err("Invalid slice indices".to_string());
            }

            let start_idx = start as usize;
            let end_idx = (end as usize).min(elements.len());

            Ok(Value::Array(elements[start_idx..end_idx].to_vec()))
        }
        "join" => {
            if evaluated_args.len() != 1 {
                return Err(format!(
                    "join() expects 1 argument, got {}",
                    evaluated_args.len()
                ));
            }
            let delimiter = match &evaluated_args[0] {
                Value::String(d) => d,
                _ => return Err("join() delimiter must be string".to_string()),
            };

            let strings: Vec<String> = elements.iter().map(|v| v.to_string()).collect();

            Ok(Value::String(strings.join(delimiter)))
        }
        _ => Err(format!("Unknown method '{}' for array", method_name)),
    }
}
