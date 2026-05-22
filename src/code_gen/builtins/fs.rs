use super::super::{Interpreter, Value};
use crate::ast::Expression;
use std::fs;
use std::path::Path;

pub(super) fn call(
    interp: &mut Interpreter,
    name: &str,
    args: &[Expression],
) -> Result<Option<Value>, String> {
    match name {
        "read_file" => {
            if args.len() != 1 {
                return Err(format!(
                    "read_file() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let filename = interp.eval_expression(&args[0])?;
            if let Value::String(filename_str) = filename {
                match fs::read_to_string(&filename_str) {
                    Ok(content) => Ok(Some(Value::String(content))),
                    Err(e) => Err(format!("Error reading file '{}': {}", filename_str, e)),
                }
            } else {
                Err("read_file() filename must be a string".to_string())
            }
        }

        "write_file" => {
            if args.len() != 2 {
                return Err(format!(
                    "write_file() expects 2 arguments, got {}",
                    args.len()
                ));
            }

            let filename = interp.eval_expression(&args[0])?;
            let content = interp.eval_expression(&args[1])?;

            if let Value::String(filename_str) = filename {
                let content_str = match content {
                    Value::String(s) => s,
                    other => other.to_string(),
                };

                let processed_content = content_str.replace("\\n", "\n");

                match fs::write(&filename_str, processed_content) {
                    Ok(_) => Ok(Some(Value::Void)),
                    Err(e) => Err(format!("Error writing file '{}': {}", filename_str, e)),
                }
            } else {
                Err("write_file() filename must be a string".to_string())
            }
        }

        "append_file" => {
            if args.len() != 2 {
                return Err(format!(
                    "append_file() expects 2 arguments, got {}",
                    args.len()
                ));
            }

            let filename = interp.eval_expression(&args[0])?;
            let content = interp.eval_expression(&args[1])?;

            if let Value::String(filename_str) = filename {
                let content_str = match content {
                    Value::String(s) => s,
                    other => other.to_string(),
                };

                let processed_content = content_str.replace("\\n", "\n");

                match fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&filename_str)
                {
                    Ok(mut file) => {
                        use std::io::Write;
                        match file.write_all(processed_content.as_bytes()) {
                            Ok(_) => Ok(Some(Value::Void)),
                            Err(e) => {
                                Err(format!("Error appending to file '{}': {}", filename_str, e))
                            }
                        }
                    }
                    Err(e) => Err(format!("Error opening file '{}': {}", filename_str, e)),
                }
            } else {
                Err("append_file() filename must be a string".to_string())
            }
        }

        "file_exists" => {
            if args.len() != 1 {
                return Err(format!(
                    "file_exists() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let filename = interp.eval_expression(&args[0])?;
            if let Value::String(filename_str) = filename {
                let exists = Path::new(&filename_str).exists();
                Ok(Some(Value::Bool(exists)))
            } else {
                Err("file_exists() filename must be a string".to_string())
            }
        }

        "list_directory" => {
            if args.len() != 1 {
                return Err(format!(
                    "list_directory() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let path_val = interp.eval_expression(&args[0])?;
            if let Value::String(path_str) = path_val {
                match fs::read_dir(&path_str) {
                    Ok(entries) => {
                        let names: Vec<Value> = entries
                            .filter_map(|e| e.ok())
                            .filter_map(|e| e.file_name().into_string().ok())
                            .map(Value::String)
                            .collect();
                        Ok(Some(Value::Array(names)))
                    }
                    Err(_) => Ok(Some(Value::Array(vec![]))),
                }
            } else {
                Err("list_directory() path must be a string".to_string())
            }
        }

        "create_directory" => {
            if args.len() != 1 {
                return Err(format!(
                    "create_directory() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let path_val = interp.eval_expression(&args[0])?;
            if let Value::String(path_str) = path_val {
                match fs::create_dir_all(&path_str) {
                    Ok(_) => Ok(Some(Value::Bool(true))),
                    Err(_) => Ok(Some(Value::Bool(false))),
                }
            } else {
                Err("create_directory() path must be a string".to_string())
            }
        }

        "remove_file" => {
            if args.len() != 1 {
                return Err(format!(
                    "remove_file() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let path_val = interp.eval_expression(&args[0])?;
            if let Value::String(path_str) = path_val {
                match fs::remove_file(&path_str) {
                    Ok(_) => Ok(Some(Value::Bool(true))),
                    Err(_) => Ok(Some(Value::Bool(false))),
                }
            } else {
                Err("remove_file() path must be a string".to_string())
            }
        }

        "remove_directory" => {
            if args.len() != 1 {
                return Err(format!(
                    "remove_directory() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let path_val = interp.eval_expression(&args[0])?;
            if let Value::String(path_str) = path_val {
                match fs::remove_dir_all(&path_str) {
                    Ok(_) => Ok(Some(Value::Bool(true))),
                    Err(_) => Ok(Some(Value::Bool(false))),
                }
            } else {
                Err("remove_directory() path must be a string".to_string())
            }
        }

        "get_file_size" => {
            if args.len() != 1 {
                return Err(format!(
                    "get_file_size() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let path_val = interp.eval_expression(&args[0])?;
            if let Value::String(path_str) = path_val {
                match fs::metadata(&path_str) {
                    Ok(meta) => Ok(Some(Value::Int(meta.len() as i64))),
                    Err(_) => Ok(Some(Value::Int(0))),
                }
            } else {
                Err("get_file_size() path must be a string".to_string())
            }
        }

        "is_dir" => {
            if args.len() != 1 {
                return Err(format!("is_dir() expects 1 argument, got {}", args.len()));
            }

            let path_val = interp.eval_expression(&args[0])?;
            if let Value::String(path_str) = path_val {
                let is_dir = Path::new(&path_str).is_dir();
                Ok(Some(Value::Bool(is_dir)))
            } else {
                Err("is_dir() path must be a string".to_string())
            }
        }

        _ => Ok(None),
    }
}
