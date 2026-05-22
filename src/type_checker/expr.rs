use super::{Type, TypeChecker};
use crate::ast::{Expression, Operator};

impl TypeChecker {
    pub fn check_expression(&mut self, expr: &Expression) -> Result<Type, String> {
        self.check_expression_with_expected_type(expr, None)
    }

    pub(super) fn check_expression_with_expected_type(
        &mut self,
        expr: &Expression,
        expected_type: Option<&Type>,
    ) -> Result<Type, String> {
        match expr {
            Expression::Uninitialized => {
                if let Some(et) = expected_type {
                    match et {
                        Type::Struct(struct_name) => {
                            if self.structs.contains_key(struct_name) {
                                Ok(Type::Struct(struct_name.clone()))
                            } else {
                                Err(format!(
                                    "Struct '{}' is not defined\n   = help: Declare it with 'struct {} {{ ... }}' before use.",
                                    struct_name, struct_name
                                ))
                            }
                        }
                        _ => Err(
                            "Uninitialized declaration (`let x: T;`) is only allowed for struct types\n   = help: Use `let x: T = value;` for primitives and arrays, or provide a struct initializer."
                                .to_string(),
                        ),
                    }
                } else {
                    Err(
                        "Invalid use of uninitialized value (internal error)\n   = help: This should only appear in `let name: StructType;`."
                            .to_string(),
                    )
                }
            }

            Expression::Integer(_) => Ok(Type::Int),
            Expression::Float(_) => Ok(Type::Float),
            Expression::Boolean(_) => Ok(Type::Bool),
            Expression::StringLiteral(_) => Ok(Type::String),

            Expression::Identifier(name) => {
                if let Some(var_type) = self.variables.get(name) {
                    Ok(var_type.clone())
                } else {
                    Err(format!(
                        "Variable '{}' not declared\n   = help: Declare it with 'let {}: type = value;' before using it.",
                        name, name
                    ))
                }
            }

            Expression::UnaryOp(op, expr) => {
                let expr_type = self.check_expression(expr)?;

                match op {
                    Operator::UnaryMinus => match expr_type {
                        Type::Int | Type::Float => Ok(expr_type),
                        _ => Err(format!("Cannot apply unary minus to {:?}", expr_type)),
                    },
                    Operator::Not => {
                        if expr_type == Type::Bool {
                            Ok(Type::Bool)
                        } else {
                            Err(format!("Cannot apply logical not to {:?}", expr_type))
                        }
                    }
                    _ => Err(format!("Unknown unary operator: {:?}", op)),
                }
            }

            Expression::BinaryOp(left, op, right) => self.check_binop(left, op, right),

            Expression::FunctionCall(name, args) => {
                if let Some(return_type) = self.check_builtin_function(name, args)? {
                    return Ok(return_type);
                }

                if let Some((return_type, param_types)) = self.functions.get(name).cloned() {
                    if args.len() != param_types.len() {
                        let sig: String = param_types
                            .iter()
                            .map(|t| t.fmt_for_user())
                            .collect::<Vec<_>>()
                            .join(", ");
                        return Err(format!(
                            "Function '{}' expects {} arguments, got {}\n   = help: Expected signature: {}({})",
                            name,
                            param_types.len(),
                            args.len(),
                            name,
                            sig
                        ));
                    }

                    for (i, arg) in args.iter().enumerate() {
                        let arg_type = self.check_expression(arg)?;
                        if arg_type != param_types[i] {
                            let expected = param_types[i].fmt_for_user();
                            let got = arg_type.fmt_for_user();
                            return Err(format!(
                                "Function '{}' parameter {} expects {}, got {}\n   = help: Pass a value of type '{}' for this parameter.",
                                name,
                                i + 1,
                                expected,
                                got,
                                expected
                            ));
                        }
                    }

                    Ok(return_type)
                } else {
                    Err(format!(
                        "Function '{}' not declared\n   = help: Define the function with 'fun {} (...) -> returnType {{ ... }}' or check the name.",
                        name, name
                    ))
                }
            }

            Expression::ArrayLiteral(elements) => {
                if elements.is_empty() {
                    if let Some(et) = expected_type {
                        if let Type::Array(element_type) = et {
                            return Ok(Type::Array(element_type.clone()));
                        }
                    }
                    return Err(
                        "Cannot infer type of empty array\n   = help: Give the array an explicit type, e.g. let arr: int[] = []; or add at least one element.".to_string(),
                    );
                }

                let elem_expected = match expected_type {
                    Some(Type::Array(inner)) => Some(inner.as_ref()),
                    _ => None,
                };

                let first_type =
                    self.check_expression_with_expected_type(&elements[0], elem_expected)?;
                for element in elements.iter().skip(1) {
                    let element_type =
                        self.check_expression_with_expected_type(element, elem_expected)?;
                    if element_type != first_type {
                        return Err(format!(
                            "Array elements must have the same type, got {} and {}\n   = help: All elements in [a, b, c, ...] must be the same type. Use separate arrays or convert values.",
                            first_type.fmt_for_user(),
                            element_type.fmt_for_user()
                        ));
                    }
                }

                Ok(Type::Array(Box::new(first_type)))
            }

            Expression::ArrayIndex(array_expr, index_expr) => {
                let index_type = self.check_expression(index_expr)?;
                if index_type != Type::Int {
                    return Err(format!(
                        "Array index must be integer, got {}\n   = help: Use an int expression, e.g. arr[i] where i is int.",
                        index_type.fmt_for_user()
                    ));
                }

                let array_type = self.check_expression(array_expr)?;
                match array_type {
                    Type::Array(element_type) => Ok(*element_type),
                    Type::String => Ok(Type::String),
                    _ => Err("Cannot index non-array or non-string value".to_string()),
                }
            }

            Expression::MethodCall(object_expr, method_name, args) => {
                self.check_method_call(object_expr, method_name, args)
            }

            Expression::StructInstantiation(struct_name, fields) => {
                let struct_info_clone = match self.structs.get(struct_name) {
                    Some(s) => s.clone(),
                    None => {
                        return Err(format!(
                            "Struct '{}' not found\n   = help: Define it with 'struct {} {{ field: type, ... }}' or check the name.",
                            struct_name, struct_name
                        ))
                    }
                };

                for (field_name, field_value) in fields {
                    if let Some(expected_type) = struct_info_clone.fields.get(field_name) {
                        let actual_type = self.check_expression_with_expected_type(
                            field_value,
                            Some(expected_type),
                        )?;
                        if actual_type != *expected_type {
                            return Err(format!(
                                    "Field '{}' in struct '{}' expects {}, got {}\n   = help: Use a value of type '{}' for this field.",
                                    field_name,
                                    struct_name,
                                    expected_type.fmt_for_user(),
                                    actual_type.fmt_for_user(),
                                    expected_type.fmt_for_user()
                                ));
                        }
                    } else {
                        let available: Vec<&str> = struct_info_clone
                            .fields
                            .keys()
                            .map(String::as_str)
                            .collect();
                        return Err(format!(
                            "Field '{}' not found in struct '{}'\n   = help: Available fields: {}",
                            field_name,
                            struct_name,
                            available.join(", ")
                        ));
                    }
                }

                for field_name in struct_info_clone.fields.keys() {
                    if !fields.iter().any(|(name, _)| name == field_name) {
                        return Err(format!(
                            "Missing required field '{}' in struct '{}'",
                            field_name, struct_name
                        ));
                    }
                }

                Ok(Type::Struct(struct_name.clone()))
            }

            Expression::FieldAccess(object_expr, field_name) => {
                let object_type = self.check_expression(object_expr)?;

                if let Type::Struct(struct_name) = object_type {
                    if let Some(struct_info) = self.structs.get(&struct_name) {
                        if let Some(field_type) = struct_info.fields.get(field_name) {
                            Ok(field_type.clone())
                        } else {
                            let available: Vec<&str> =
                                struct_info.fields.keys().map(String::as_str).collect();
                            Err(format!(
                                "Field '{}' not found in struct '{}'\n   = help: Available fields: {}",
                                field_name,
                                struct_name,
                                available.join(", ")
                            ))
                        }
                    } else {
                        Err(format!(
                                "Struct '{}' not found\n   = help: Define it with 'struct {} {{ field: type, ... }}' or check the name.",
                                struct_name, struct_name
                            ))
                    }
                } else {
                    Err(format!(
                        "Cannot access field on non-struct value of type '{}'\n   = help: Only struct values have fields.",
                        object_type.fmt_for_user()
                    ))
                }
            }

            Expression::EnumVariant(enum_name, variant_name) => {
                if let Some(enum_info) = self.enums.get(enum_name) {
                    if enum_info.variants.contains(variant_name) {
                        Ok(Type::Enum(enum_name.clone()))
                    } else {
                        let available = enum_info.variants.join(", ");
                        Err(format!(
                            "Variant '{}' not found in enum '{}'\n   = help: Available variants: {}",
                            variant_name, enum_name, available
                        ))
                    }
                } else {
                    Err(format!(
                        "Enum '{}' not found\n   = help: Define it with 'enum {} {{ Variant1, Variant2, ... }}' or check the name.",
                        enum_name, enum_name
                    ))
                }
            }
        }
    }
}
