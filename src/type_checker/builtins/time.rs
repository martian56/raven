use super::super::{Type, TypeChecker};
use crate::ast::Expression;

pub(super) fn check(
    _tc: &mut TypeChecker,
    name: &str,
    args: &[Expression],
) -> Result<Option<Type>, String> {
    match name {
        "sys_time" | "sys_date" => {
            if !args.is_empty() {
                return Err(format!(
                    "{}() expects 0 arguments, got {}",
                    name,
                    args.len()
                ));
            }

            Ok(Some(Type::String))
        }

        "sys_timestamp" => {
            if !args.is_empty() {
                return Err(format!(
                    "sys_timestamp() expects 0 arguments, got {}",
                    args.len()
                ));
            }

            Ok(Some(Type::Float))
        }

        _ => Ok(None),
    }
}
