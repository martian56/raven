use super::super::{Interpreter, Value};
use crate::ast::Expression;
use std::io::{self, Write};

pub(super) fn call(
    interp: &mut Interpreter,
    name: &str,
    args: &[Expression],
) -> Result<Option<Value>, String> {
    match name {
        "print" => {
            if args.is_empty() {
                return Err("print() expects at least 1 argument".to_string());
            }

            if args.len() == 1 {
                let value = interp.eval_expression(&args[0])?;
                println!("{}", value);
            } else {
                let format_string = interp.eval_expression(&args[0])?;
                if let Value::String(format_str) = format_string {
                    let mut formatted = format_str.clone();

                    for (i, arg) in args.iter().enumerate().skip(1) {
                        let arg_value = interp.eval_expression(arg)?;
                        let placeholder = "{}";
                        if let Some(pos) = formatted.find(placeholder) {
                            formatted.replace_range(
                                pos..pos + placeholder.len(),
                                &arg_value.to_string(),
                            );
                        } else {
                            return Err(format!("Too many arguments for print() - format string has no placeholder for argument {}", i));
                        }
                    }

                    if formatted.contains("{}") {
                        return Err("Too few arguments for print() - format string has unmatched placeholders".to_string());
                    }

                    println!("{}", formatted);
                } else {
                    return Err("print() format string must be a string".to_string());
                }
            }

            Ok(Some(Value::Void))
        }

        "input" => {
            if args.len() > 1 {
                return Err(format!(
                    "input() expects 0 or 1 argument, got {}",
                    args.len()
                ));
            }

            if args.len() == 1 {
                let prompt = interp.eval_expression(&args[0])?;
                if let Value::String(prompt_str) = prompt {
                    print!("{}", prompt_str);
                    io::stdout().flush().unwrap();
                } else {
                    return Err("input() prompt must be a string".to_string());
                }
            }

            let mut input = String::new();
            match io::stdin().read_line(&mut input) {
                Ok(_) => {
                    input = input.trim().to_string();
                    Ok(Some(Value::String(input)))
                }
                Err(e) => Err(format!("Error reading input: {}", e)),
            }
        }

        "format" => {
            if args.is_empty() {
                return Err(format!(
                    "format() expects at least 1 argument, got {}",
                    args.len()
                ));
            }

            let template = interp.eval_expression(&args[0])?;
            if let Value::String(template_str) = template {
                let mut result = template_str.clone();
                let mut arg_index = 1;

                while let Some(pos) = result.find("{}") {
                    if arg_index >= args.len() {
                        return Err("format() not enough arguments for placeholders".to_string());
                    }

                    let replacement_value = interp.eval_expression(&args[arg_index])?;
                    let replacement = replacement_value.to_string();
                    result.replace_range(pos..pos + 2, &replacement);
                    arg_index += 1;
                }

                Ok(Some(Value::String(result)))
            } else {
                Err("format() template must be a string".to_string())
            }
        }

        _ => Ok(None),
    }
}
