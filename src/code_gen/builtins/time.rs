use super::super::{Interpreter, Value};
use crate::ast::Expression;
use chrono::Local;

pub(super) fn call(
    _interp: &mut Interpreter,
    name: &str,
    args: &[Expression],
) -> Result<Option<Value>, String> {
    match name {
        "sys_time" => {
            if !args.is_empty() {
                return Err(format!(
                    "sys_time() expects 0 arguments, got {}",
                    args.len()
                ));
            }

            let now = Local::now();
            Ok(Some(Value::String(now.format("%H:%M:%S").to_string())))
        }

        "sys_date" => {
            if !args.is_empty() {
                return Err(format!(
                    "sys_date() expects 0 arguments, got {}",
                    args.len()
                ));
            }

            let now = Local::now();
            Ok(Some(Value::String(now.format("%Y-%m-%d").to_string())))
        }

        "sys_timestamp" => {
            if !args.is_empty() {
                return Err(format!(
                    "sys_timestamp() expects 0 arguments, got {}",
                    args.len()
                ));
            }

            let now = Local::now();
            Ok(Some(Value::Float(
                now.timestamp() as f64 + now.timestamp_subsec_millis() as f64 / 1000.0,
            )))
        }

        _ => Ok(None),
    }
}
