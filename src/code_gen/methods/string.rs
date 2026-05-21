use super::super::{Interpreter, Value};

pub(super) fn call(
    _interp: &mut Interpreter,
    s: &str,
    method_name: &str,
    evaluated_args: &[Value],
) -> Result<Value, String> {
    match method_name {
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

            if start < 0 || end < 0 || start > end || start as usize >= s.len() {
                return Err("Invalid slice indices".to_string());
            }

            let start_idx = start as usize;
            let end_idx = (end as usize).min(s.len());

            Ok(Value::String(s[start_idx..end_idx].to_string()))
        }
        "split" => {
            if evaluated_args.len() != 1 {
                return Err(format!(
                    "split() expects 1 argument, got {}",
                    evaluated_args.len()
                ));
            }
            let delimiter = match &evaluated_args[0] {
                Value::String(d) => d,
                _ => return Err("split() delimiter must be string".to_string()),
            };

            let parts: Vec<Value> = s
                .split(delimiter.as_str())
                .map(|part| Value::String(part.to_string()))
                .collect();

            Ok(Value::Array(parts))
        }
        "replace" => {
            if evaluated_args.len() != 2 {
                return Err(format!(
                    "replace() expects 2 arguments, got {}",
                    evaluated_args.len()
                ));
            }
            let from = match &evaluated_args[0] {
                Value::String(f) => f,
                _ => return Err("replace() 'from' must be string".to_string()),
            };
            let to = match &evaluated_args[1] {
                Value::String(t) => t,
                _ => return Err("replace() 'to' must be string".to_string()),
            };

            Ok(Value::String(s.replace(from.as_str(), to.as_str())))
        }
        "index_of" => {
            if evaluated_args.len() != 1 {
                return Err(format!(
                    "index_of() expects 1 argument, got {}",
                    evaluated_args.len()
                ));
            }
            let sub = match &evaluated_args[0] {
                Value::String(x) => x.as_str(),
                _ => return Err("index_of() argument must be string".to_string()),
            };
            let i = s.find(sub).map(|i| i as i64).unwrap_or(-1);
            Ok(Value::Int(i))
        }
        "last_index_of" => {
            if evaluated_args.len() != 1 {
                return Err(format!(
                    "last_index_of() expects 1 argument, got {}",
                    evaluated_args.len()
                ));
            }
            let sub = match &evaluated_args[0] {
                Value::String(x) => x.as_str(),
                _ => return Err("last_index_of() argument must be string".to_string()),
            };
            let i = s.rfind(sub).map(|i| i as i64).unwrap_or(-1);
            Ok(Value::Int(i))
        }
        _ => Err(format!("Unknown method '{}' for string", method_name)),
    }
}
