mod string;

use super::{Interpreter, Value};
use crate::ast::Expression;

impl Interpreter {
    pub(super) fn eval_method_call(
        &mut self,
        object_expr: &Expression,
        method_name: &str,
        args: &[Expression],
    ) -> Result<Value, String> {
        let mut evaluated_args = Vec::new();
        for arg in args {
            evaluated_args.push(self.eval_expression(arg)?);
        }

        // map.keys.push(x) / map.keys.pop() — mutate array stored in a struct field (not
        // only a standalone variable). Without this, push returns a new array but the Map
        // struct is never updated (collections.rv map_set/map_remove rely on this).
        if let Expression::FieldAccess(inner, field_name) = object_expr {
            if let Expression::Identifier(var_name) = inner.as_ref() {
                if method_name == "push" || method_name == "pop" {
                    if let Some(Value::Struct(_, ref mut fields)) = self.variables.get_mut(var_name)
                    {
                        if let Some(Value::Array(mut elements)) = fields.get(field_name).cloned() {
                            match method_name {
                                "push" => {
                                    if evaluated_args.len() != 1 {
                                        return Err(format!(
                                            "push() expects 1 argument, got {}",
                                            evaluated_args.len()
                                        ));
                                    }
                                    elements.push(evaluated_args[0].clone());
                                    fields
                                        .insert(field_name.clone(), Value::Array(elements.clone()));
                                    return Ok(Value::Array(elements));
                                }
                                "pop" => {
                                    if !evaluated_args.is_empty() {
                                        return Err(format!(
                                            "pop() expects 0 arguments, got {}",
                                            evaluated_args.len()
                                        ));
                                    }
                                    if elements.is_empty() {
                                        return Err("Cannot pop from empty array".to_string());
                                    }
                                    let popped = elements.pop().unwrap();
                                    fields.insert(field_name.clone(), Value::Array(elements));
                                    return Ok(popped);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        if let Expression::Identifier(var_name) = object_expr {
            if let Some(Value::Array(mut elements)) = self.variables.get(var_name).cloned() {
                match method_name {
                    "push" => {
                        if evaluated_args.len() != 1 {
                            return Err(format!(
                                "push() expects 1 argument, got {}",
                                evaluated_args.len()
                            ));
                        }
                        elements.push(evaluated_args[0].clone());
                        self.variables
                            .insert(var_name.clone(), Value::Array(elements.clone()));
                        Ok(Value::Array(elements))
                    }
                    "pop" => {
                        if !evaluated_args.is_empty() {
                            return Err(format!(
                                "pop() expects 0 arguments, got {}",
                                evaluated_args.len()
                            ));
                        }
                        if elements.is_empty() {
                            return Err("Cannot pop from empty array".to_string());
                        }
                        let popped = elements.pop().unwrap();
                        self.variables
                            .insert(var_name.clone(), Value::Array(elements));
                        Ok(popped)
                    }
                    "slice" => {
                        if evaluated_args.len() != 2 {
                            return Err(format!(
                                "slice() expects 2 arguments, got {}",
                                evaluated_args.len()
                            ));
                        }
                        let start = match &evaluated_args[0] {
                            Value::Int(i) => *i,
                            _ => return Err("slice() start index must be integer".to_string()),
                        };
                        let end = match &evaluated_args[1] {
                            Value::Int(i) => *i,
                            _ => return Err("slice() end index must be integer".to_string()),
                        };

                        if start < 0 || end < 0 || start > end || start as usize >= elements.len() {
                            return Err("Invalid slice indices".to_string());
                        }

                        let start_idx = start as usize;
                        let end_idx = (end as usize).min(elements.len());

                        Ok(Value::Array(elements[start_idx..end_idx].to_vec()))
                    }
                    "join" => {
                        if evaluated_args.len() != 1 {
                            return Err(format!(
                                "join() expects 1 argument, got {}",
                                evaluated_args.len()
                            ));
                        }
                        let delimiter = match &evaluated_args[0] {
                            Value::String(d) => d,
                            _ => return Err("join() delimiter must be string".to_string()),
                        };

                        let strings: Vec<String> = elements.iter().map(|v| v.to_string()).collect();

                        Ok(Value::String(strings.join(delimiter)))
                    }
                    _ => Err(format!("Unknown method '{}' for array", method_name)),
                }
            } else if let Some(module_name_clone) =
                self.variables.get(var_name).and_then(|v| match v {
                    Value::Module(name) => Some(name.clone()),
                    _ => None,
                })
            {
                if !self.modules.contains_key(&module_name_clone) {
                    self.load_module(&module_name_clone)?;
                }
                if let Some(module) = self.modules.get(&module_name_clone) {
                    if let Some(func) = module.functions.get(method_name) {
                        let func_clone = func.clone();
                        let module_clone = module.clone();
                        self.call_function_with_module(&func_clone, evaluated_args, &module_clone)
                    } else if let Some(value) = module.variables.get(method_name) {
                        Ok(value.clone())
                    } else {
                        let available: Vec<String> = module
                            .functions
                            .keys()
                            .chain(module.variables.keys())
                            .cloned()
                            .collect();
                        Err(format!(
                            "Method '{}' not found in module '{}'\n   = help: Available: {}",
                            method_name,
                            module_name_clone,
                            available.join(", ")
                        ))
                    }
                } else {
                    Err(format!("Module '{}' not found", module_name_clone))
                }
            } else if let Some(Value::String(s)) = self.variables.get(var_name).cloned() {
                string::call(self, &s, method_name, &evaluated_args)
            } else if let Some(Value::Struct(struct_name, fields)) =
                self.variables.get(var_name).cloned()
            {
                let struct_val = Value::Struct(struct_name, fields);
                self.call_struct_method(
                    struct_val,
                    method_name,
                    evaluated_args,
                    Some(var_name.clone()),
                )
            } else {
                Err(format!(
                    "Variable '{}' is not an array, module, string, or struct with methods",
                    var_name
                ))
            }
        } else {
            let object = self.eval_expression(object_expr)?;

            if let Value::Array(elements) = object {
                match method_name {
                    "push" => {
                        if evaluated_args.len() != 1 {
                            return Err(format!(
                                "push() expects 1 argument, got {}",
                                evaluated_args.len()
                            ));
                        }
                        let mut new_elements = elements.clone();
                        new_elements.push(evaluated_args[0].clone());
                        Ok(Value::Array(new_elements))
                    }
                    "pop" => {
                        if !evaluated_args.is_empty() {
                            return Err(format!(
                                "pop() expects 0 arguments, got {}",
                                evaluated_args.len()
                            ));
                        }
                        if elements.is_empty() {
                            return Err("Cannot pop from empty array".to_string());
                        }
                        Ok(elements.last().unwrap().clone())
                    }
                    "slice" => {
                        if evaluated_args.len() != 2 {
                            return Err(format!(
                                "slice() expects 2 arguments, got {}",
                                evaluated_args.len()
                            ));
                        }
                        let start = match &evaluated_args[0] {
                            Value::Int(i) => *i,
                            _ => return Err("slice() start index must be integer".to_string()),
                        };
                        let end = match &evaluated_args[1] {
                            Value::Int(i) => *i,
                            _ => return Err("slice() end index must be integer".to_string()),
                        };

                        if start < 0 || end < 0 || start > end || start as usize >= elements.len() {
                            return Err("Invalid slice indices".to_string());
                        }

                        let start_idx = start as usize;
                        let end_idx = (end as usize).min(elements.len());

                        Ok(Value::Array(elements[start_idx..end_idx].to_vec()))
                    }
                    "join" => {
                        if evaluated_args.len() != 1 {
                            return Err(format!(
                                "join() expects 1 argument, got {}",
                                evaluated_args.len()
                            ));
                        }
                        let delimiter = match &evaluated_args[0] {
                            Value::String(d) => d,
                            _ => return Err("join() delimiter must be string".to_string()),
                        };

                        let strings: Vec<String> = elements.iter().map(|v| v.to_string()).collect();

                        Ok(Value::String(strings.join(delimiter)))
                    }
                    _ => Err(format!("Unknown method '{}' for array", method_name)),
                }
            } else if let Value::String(s) = object {
                string::call(self, &s, method_name, &evaluated_args)
            } else if let Value::Struct(..) = &object {
                self.call_struct_method(object, method_name, evaluated_args, None)
            } else {
                Err(format!("Cannot call method on value of type {:?}", object))
            }
        }
    }
}
