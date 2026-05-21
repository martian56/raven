use super::super::{Interpreter, Value};
use crate::ast::Expression;

pub(super) fn call(
    interp: &mut Interpreter,
    name: &str,
    args: &[Expression],
) -> Result<Option<Value>, String> {
    match name {
        "parse_int" => {
            if args.len() != 1 {
                return Err(format!(
                    "parse_int() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let val = interp.eval_expression(&args[0])?;
            if let Value::String(s) = val {
                match s.parse::<i64>() {
                    Ok(n) => Ok(Some(Value::Int(n))),
                    Err(_) => Ok(Some(Value::Int(0))),
                }
            } else {
                Err("parse_int() expects a string argument".to_string())
            }
        }

        "char_code" => {
            if args.len() != 1 {
                return Err(format!(
                    "char_code() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let val = interp.eval_expression(&args[0])?;
            if let Value::String(s) = val {
                let code = s.chars().next().map(|c| c as i64).unwrap_or(0);
                Ok(Some(Value::Int(code)))
            } else {
                Err("char_code() expects a string argument".to_string())
            }
        }

        _ => Ok(None),
    }
}
