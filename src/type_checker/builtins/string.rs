use super::super::{Type, TypeChecker};
use crate::ast::Expression;

pub(super) fn check(
    tc: &mut TypeChecker,
    name: &str,
    args: &[Expression],
) -> Result<Option<Type>, String> {
    match name {
        "parse_int" => {
            if args.len() != 1 {
                return Err(format!(
                    "parse_int() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let arg_type = tc.check_expression(&args[0])?;
            if arg_type != Type::String {
                return Err("parse_int() expects a string argument".to_string());
            }

            Ok(Some(Type::Int))
        }

        "char_code" => {
            if args.len() != 1 {
                return Err(format!(
                    "char_code() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let arg_type = tc.check_expression(&args[0])?;
            if arg_type != Type::String {
                return Err("char_code() expects a string argument".to_string());
            }

            Ok(Some(Type::Int))
        }

        _ => Ok(None),
    }
}
