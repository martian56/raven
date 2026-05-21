mod array;
mod string;

use super::{Type, TypeChecker};
use crate::ast::Expression;

impl TypeChecker {
    pub(super) fn check_method_call(
        &mut self,
        object_expr: &Expression,
        method_name: &str,
        args: &[Expression],
    ) -> Result<Type, String> {
        let object_type = self.check_expression(object_expr)?;

        if let Type::Array(_) = object_type {
            array::check(self, object_type, method_name, args)
        } else if let Type::Module = object_type {
            if let Expression::Identifier(module_var) = object_expr {
                if let Some(module_name) = self.module_bindings.get(module_var) {
                    if let Some(module_info) = self.modules.get(module_name) {
                        if let Some((return_type, param_types)) =
                            module_info.functions.get(method_name).cloned()
                        {
                            if args.len() != param_types.len() {
                                return Err(format!(
                                    "Function '{}.{}' expects {} arguments, got {}\n   = help: Expected signature: {}.{}({})",
                                    module_var,
                                    method_name,
                                    param_types.len(),
                                    args.len(),
                                    module_var,
                                    method_name,
                                    param_types
                                        .iter()
                                        .map(|t| t.fmt_for_user())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ));
                            }

                            for (i, arg) in args.iter().enumerate() {
                                let arg_type = self.check_expression(arg)?;
                                if arg_type != param_types[i] {
                                    return Err(format!(
                                        "Function '{}.{}' parameter {} expects {}, got {}\n   = help: Pass a value of type '{}' for this parameter.",
                                        module_var,
                                        method_name,
                                        i + 1,
                                        param_types[i].fmt_for_user(),
                                        arg_type.fmt_for_user(),
                                        param_types[i].fmt_for_user()
                                    ));
                                }
                            }

                            return Ok(return_type);
                        } else {
                            let available = module_info
                                .functions
                                .keys()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Err(format!(
                                "Function '{}.{}' not found\n   = help: Available module functions: {}",
                                module_var,
                                method_name,
                                if available.is_empty() {
                                    "(none)".to_string()
                                } else {
                                    available
                                }
                            ));
                        }
                    }
                }
            }
            Ok(Type::Unknown)
        } else if let Type::String = object_type {
            string::check(self, Type::String, method_name, args)
        } else if let Type::Struct(struct_name) = object_type {
            let (return_type, param_types) = match self.struct_methods.get(&struct_name) {
                Some(methods) => match methods.get(method_name) {
                    Some((ret, params)) => (ret.clone(), params.clone()),
                    None => {
                        let available: Vec<String> = methods.keys().cloned().collect();
                        return Err(format!(
                            "Method '{}' not found on struct '{}'\n   = help: Available methods: {}",
                            method_name,
                            struct_name,
                            available.join(", ")
                        ));
                    }
                },
                None => {
                    return Err(format!(
                        "Struct '{}' has no methods defined\n   = help: Add methods with 'impl {} {{ fun method(self, ...) {{ ... }} }}'",
                        struct_name, struct_name
                    ));
                }
            };
            if args.len() + 1 != param_types.len() {
                return Err(format!(
                    "Method '{}' on '{}' expects {} arguments (including self), got {}\n   = help: Expected signature: {}({})",
                    method_name,
                    struct_name,
                    param_types.len() - 1,
                    args.len(),
                    method_name,
                    param_types
                        .iter()
                        .skip(1)
                        .map(|t| t.fmt_for_user())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            for (i, arg) in args.iter().enumerate() {
                let arg_type = self.check_expression(arg)?;
                let expected = &param_types[i + 1];
                if arg_type != *expected {
                    return Err(format!(
                        "Method '{}' argument {} expects {}, got {}",
                        method_name,
                        i + 1,
                        expected.fmt_for_user(),
                        arg_type.fmt_for_user()
                    ));
                }
            }
            Ok(return_type)
        } else {
            Err(format!(
                "Cannot call method on value of type '{}'\n   = help: Methods work on arrays, strings, modules, and structs with impl blocks.",
                object_type.fmt_for_user()
            ))
        }
    }
}
