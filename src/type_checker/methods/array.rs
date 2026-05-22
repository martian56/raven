use super::super::{Type, TypeChecker};
use crate::ast::Expression;

/// Type-check method calls whose receiver is a `Type::Array(_)`.
pub(super) fn check(
    tc: &mut TypeChecker,
    receiver_type: Type,
    method_name: &str,
    args: &[Expression],
) -> Result<Type, String> {
    let element_type = match receiver_type {
        Type::Array(et) => et,
        _ => unreachable!("array::check called with non-array receiver type"),
    };

    match method_name {
        "push" => {
            if args.len() != 1 {
                return Err(format!("push() expects 1 argument, got {}", args.len()));
            }
            let arg_type = tc.check_expression(&args[0])?;
            if arg_type != *element_type {
                return Err(format!(
                    "push() argument type mismatch: expected {:?}, got {:?}",
                    element_type, arg_type
                ));
            }
            Ok(Type::Array(element_type)) // push() returns the modified array
        }
        "pop" => {
            if !args.is_empty() {
                return Err(format!("pop() expects 0 arguments, got {}", args.len()));
            }
            Ok(*element_type)
        }
        "slice" => {
            if args.len() != 2 {
                return Err(format!("slice() expects 2 arguments, got {}", args.len()));
            }
            let start_type = tc.check_expression(&args[0])?;
            let end_type = tc.check_expression(&args[1])?;
            if start_type != Type::Int || end_type != Type::Int {
                return Err("slice() arguments must be integers".to_string());
            }
            Ok(Type::Array(element_type))
        }
        "join" => {
            if args.len() != 1 {
                return Err(format!("join() expects 1 argument, got {}", args.len()));
            }
            let delimiter_type = tc.check_expression(&args[0])?;
            if delimiter_type != Type::String {
                return Err("join() delimiter must be string".to_string());
            }
            Ok(Type::String)
        }
        _ => Err(format!("Unknown method '{}' for array", method_name)),
    }
}
