mod array;
mod module;
mod string;
mod struct_impl;
mod tcp;

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
            if let Some(Value::Array(elements)) = self.variables.get(var_name).cloned() {
                let name = var_name.clone();
                array::call(self, elements, method_name, &evaluated_args, Some(&name))
            } else if let Some(module_name_clone) =
                self.variables.get(var_name).and_then(|v| match v {
                    Value::Module(name) => Some(name.clone()),
                    _ => None,
                })
            {
                module::call(self, &module_name_clone, method_name, evaluated_args)
            } else if let Some(Value::String(s)) = self.variables.get(var_name).cloned() {
                string::call(self, &s, method_name, &evaluated_args)
            } else if let Some(v @ (Value::TcpListener(_) | Value::TcpStream(_))) =
                self.variables.get(var_name).cloned()
            {
                tcp::call(self, v, method_name, &evaluated_args)
            } else if let Some(Value::Struct(struct_name, fields)) =
                self.variables.get(var_name).cloned()
            {
                let struct_val = Value::Struct(struct_name, fields);
                struct_impl::call(
                    self,
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
                array::call(self, elements, method_name, &evaluated_args, None)
            } else if let Value::String(s) = object {
                string::call(self, &s, method_name, &evaluated_args)
            } else if matches!(object, Value::TcpListener(_) | Value::TcpStream(_)) {
                tcp::call(self, object, method_name, &evaluated_args)
            } else if let Value::Struct(..) = &object {
                struct_impl::call(self, object, method_name, evaluated_args, None)
            } else {
                Err(format!("Cannot call method on value of type {:?}", object))
            }
        }
    }
}
