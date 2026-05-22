use super::super::{Type, TypeChecker};
use crate::ast::Expression;

pub(super) fn check(
    tc: &mut TypeChecker,
    name: &str,
    args: &[Expression],
) -> Result<Option<Type>, String> {
    match name {
        "len" => {
            if args.len() != 1 {
                return Err(format!("len() expects 1 argument, got {}", args.len()));
            }

            let arg_type = tc.check_expression(&args[0])?;
            match arg_type {
                Type::Array(_) | Type::String => Ok(Some(Type::Int)),
                _ => Err(format!("len() expects array or string, got '{}'\n   = help: len() works on arrays and strings only.", arg_type.fmt_for_user())),
            }
        }

        "type" => {
            if args.len() != 1 {
                return Err(format!("type() expects 1 argument, got {}", args.len()));
            }

            tc.check_expression(&args[0])?;
            Ok(Some(Type::String))
        }

        "panic" => {
            if args.is_empty() {
                return Err("panic() expects at least 1 argument".to_string());
            }

            for arg in args {
                tc.check_expression(arg)?;
            }

            Ok(Some(Type::Void))
        }

        "enum_from_string" => {
            if args.len() != 2 {
                return Err(format!(
                    "enum_from_string() expects 2 arguments, got {}",
                    args.len()
                ));
            }

            let enum_name_type = tc.check_expression(&args[0])?;
            let variant_name_type = tc.check_expression(&args[1])?;

            if enum_name_type != Type::String {
                return Err("enum_from_string() first argument must be a string".to_string());
            }

            if variant_name_type != Type::String {
                return Err("enum_from_string() second argument must be a string".to_string());
            }

            if let Expression::StringLiteral(enum_name) = &args[0] {
                if tc.enums.contains_key(enum_name) {
                    return Ok(Some(Type::Enum(enum_name.clone())));
                }
            }

            Ok(Some(Type::Enum("Unknown".to_string())))
        }

        _ => Ok(None),
    }
}
