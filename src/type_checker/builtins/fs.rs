use super::super::{Type, TypeChecker};
use crate::ast::Expression;

pub(super) fn check(
    tc: &mut TypeChecker,
    name: &str,
    args: &[Expression],
) -> Result<Option<Type>, String> {
    match name {
        "read_file" => {
            if args.len() != 1 {
                return Err(format!(
                    "read_file() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let filename_type = tc.check_expression(&args[0])?;
            if filename_type != Type::String {
                return Err("read_file() filename must be a string".to_string());
            }

            Ok(Some(Type::String))
        }

        "write_file" => {
            if args.len() != 2 {
                return Err(format!(
                    "write_file() expects 2 arguments, got {}",
                    args.len()
                ));
            }

            let filename_type = tc.check_expression(&args[0])?;
            if filename_type != Type::String {
                return Err("write_file() filename must be a string".to_string());
            }

            tc.check_expression(&args[1])?;

            Ok(Some(Type::Void))
        }

        "append_file" => {
            if args.len() != 2 {
                return Err(format!(
                    "append_file() expects 2 arguments, got {}",
                    args.len()
                ));
            }

            let filename_type = tc.check_expression(&args[0])?;
            if filename_type != Type::String {
                return Err("append_file() filename must be a string".to_string());
            }

            tc.check_expression(&args[1])?;

            Ok(Some(Type::Void))
        }

        "file_exists" => {
            if args.len() != 1 {
                return Err(format!(
                    "file_exists() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let filename_type = tc.check_expression(&args[0])?;
            if filename_type != Type::String {
                return Err("file_exists() filename must be a string".to_string());
            }

            Ok(Some(Type::Bool))
        }

        "list_directory" => {
            if args.len() != 1 {
                return Err(format!(
                    "list_directory() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let path_type = tc.check_expression(&args[0])?;
            if path_type != Type::String {
                return Err("list_directory() path must be a string".to_string());
            }

            Ok(Some(Type::Array(Box::new(Type::String))))
        }

        "create_directory" | "remove_file" | "remove_directory" => {
            if args.len() != 1 {
                return Err(format!("{}() expects 1 argument, got {}", name, args.len()));
            }

            let path_type = tc.check_expression(&args[0])?;
            if path_type != Type::String {
                return Err(format!("{}() path must be a string", name));
            }

            Ok(Some(Type::Bool))
        }

        "get_file_size" => {
            if args.len() != 1 {
                return Err(format!(
                    "get_file_size() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let path_type = tc.check_expression(&args[0])?;
            if path_type != Type::String {
                return Err("get_file_size() path must be a string".to_string());
            }

            Ok(Some(Type::Int))
        }

        "is_dir" => {
            if args.len() != 1 {
                return Err(format!("is_dir() expects 1 argument, got {}", args.len()));
            }

            let path_type = tc.check_expression(&args[0])?;
            if path_type != Type::String {
                return Err("is_dir() path must be a string".to_string());
            }

            Ok(Some(Type::Bool))
        }

        _ => Ok(None),
    }
}
