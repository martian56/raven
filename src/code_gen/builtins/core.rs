use super::super::{Interpreter, Value};
use crate::ast::Expression;

pub(super) fn call(
    interp: &mut Interpreter,
    name: &str,
    args: &[Expression],
) -> Result<Option<Value>, String> {
    match name {
        "len" => {
            if args.len() != 1 {
                return Err(format!("len() expects 1 argument, got {}", args.len()));
            }

            let value = interp.eval_expression(&args[0])?;
            match value {
                Value::Array(elements) => Ok(Some(Value::Int(elements.len() as i64))),
                Value::String(s) => Ok(Some(Value::Int(s.len() as i64))),
                _ => Err(format!("len() expects array or string, got {:?}", value)),
            }
        }

        "type" => {
            if args.len() != 1 {
                return Err(format!("type() expects 1 argument, got {}", args.len()));
            }

            let value = interp.eval_expression(&args[0])?;
            let type_name = match value {
                Value::Int(_) => "int".to_string(),
                Value::Float(_) => "float".to_string(),
                Value::Bool(_) => "bool".to_string(),
                Value::String(_) => "string".to_string(),
                Value::Array(_) => "array".to_string(),
                Value::Struct(name, _) => name.clone(),
                Value::Enum(name, _) => name.clone(),
                Value::Module(_) => "module".to_string(),
                Value::Void => "void".to_string(),
                Value::TcpListener(_) => "TcpListener".to_string(),
                Value::TcpStream(_) => "TcpStream".to_string(),
            };
            Ok(Some(Value::String(type_name.to_string())))
        }

        "panic" => {
            if args.is_empty() {
                return Err("panic() expects at least 1 argument".to_string());
            }
            let mut parts = Vec::new();
            for arg in args {
                let v = interp.eval_expression(arg)?;
                parts.push(v.to_string());
            }
            Err(parts.join(""))
        }

        "enum_from_string" => {
            if args.len() != 2 {
                return Err(format!(
                    "enum_from_string() expects 2 arguments, got {}",
                    args.len()
                ));
            }

            let enum_name_value = interp.eval_expression(&args[0])?;
            let variant_name_value = interp.eval_expression(&args[1])?;

            if let (Value::String(enum_name), Value::String(variant_name)) =
                (enum_name_value, variant_name_value)
            {
                if let Some(variants) = interp.enums.get(&enum_name) {
                    if variants.contains(&variant_name) {
                        Ok(Some(Value::Enum(enum_name, variant_name)))
                    } else {
                        Err(format!(
                            "Variant '{}' not found in enum '{}'. Available variants: {:?}",
                            variant_name, enum_name, variants
                        ))
                    }
                } else {
                    Err(format!("Enum '{}' not found", enum_name))
                }
            } else {
                Err("enum_from_string() expects two string arguments".to_string())
            }
        }

        _ => Ok(None),
    }
}
