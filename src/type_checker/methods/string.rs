use super::super::{Type, TypeChecker};
use crate::ast::Expression;

/// Type-check method calls whose receiver is a `Type::String`.
pub(super) fn check(
    tc: &mut TypeChecker,
    _receiver_type: Type,
    method_name: &str,
    args: &[Expression],
) -> Result<Type, String> {
    match method_name {
        "slice" => {
            if args.len() != 2 {
                return Err(format!("slice() expects 2 arguments, got {}", args.len()));
            }
            let start_type = tc.check_expression(&args[0])?;
            let end_type = tc.check_expression(&args[1])?;
            if start_type != Type::Int || end_type != Type::Int {
                return Err("slice() arguments must be integers".to_string());
            }
            Ok(Type::String)
        }
        "split" => {
            if args.len() != 1 {
                return Err(format!("split() expects 1 argument, got {}", args.len()));
            }
            let delimiter_type = tc.check_expression(&args[0])?;
            if delimiter_type != Type::String {
                return Err("split() delimiter must be string".to_string());
            }
            Ok(Type::Array(Box::new(Type::String)))
        }
        "replace" => {
            if args.len() != 2 {
                return Err(format!("replace() expects 2 arguments, got {}", args.len()));
            }
            let from_type = tc.check_expression(&args[0])?;
            let to_type = tc.check_expression(&args[1])?;
            if from_type != Type::String || to_type != Type::String {
                return Err("replace() arguments must be strings".to_string());
            }
            Ok(Type::String)
        }
        "index_of" | "last_index_of" => {
            if args.len() != 1 {
                return Err(format!(
                    "{}() expects 1 argument, got {}",
                    method_name,
                    args.len()
                ));
            }
            let sub_type = tc.check_expression(&args[0])?;
            if sub_type != Type::String {
                return Err(format!("{}() argument must be string", method_name));
            }
            Ok(Type::Int)
        }
        _ => Err(format!("Unknown method '{}' for string", method_name)),
    }
}
