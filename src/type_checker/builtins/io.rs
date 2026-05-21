use super::super::{Type, TypeChecker};
use crate::ast::Expression;

pub(super) fn check(
    tc: &mut TypeChecker,
    name: &str,
    args: &[Expression],
) -> Result<Option<Type>, String> {
    match name {
        "print" => {
            if args.is_empty() {
                return Err("print() expects at least 1 argument".to_string());
            }

            for arg in args {
                tc.check_expression(arg)?;
            }

            Ok(Some(Type::Void))
        }

        "input" => {
            if args.len() > 1 {
                return Err(format!(
                    "input() expects 0 or 1 argument, got {}",
                    args.len()
                ));
            }

            if args.len() == 1 {
                let prompt_type = tc.check_expression(&args[0])?;
                if prompt_type != Type::String {
                    return Err("input() prompt must be a string".to_string());
                }
            }

            Ok(Some(Type::String))
        }

        "format" => {
            if args.is_empty() {
                return Err(format!(
                    "format() expects at least 1 argument, got {}",
                    args.len()
                ));
            }

            let template_type = tc.check_expression(&args[0])?;
            if template_type != Type::String {
                return Err("format() template must be a string".to_string());
            }

            Ok(Some(Type::String))
        }

        _ => Ok(None),
    }
}
