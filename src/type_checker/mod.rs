use crate::ast::{Expression, Operator};
use std::collections::HashMap;
use std::fs;

mod builtins;
mod methods;
mod stmt;
mod type_repr;
pub use type_repr::Type;

pub struct TypeChecker {
    variables: HashMap<String, Type>,
    module_bindings: HashMap<String, String>,
    functions: HashMap<String, (Type, Vec<Type>)>,
    structs: HashMap<String, StructInfo>,
    struct_methods: HashMap<String, HashMap<String, (Type, Vec<Type>)>>,
    enums: HashMap<String, EnumInfo>,
    modules: HashMap<String, ModuleInfo>,
    current_function_return_type: Option<Type>,
}

#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub variables: HashMap<String, Type>,
    pub functions: HashMap<String, (Type, Vec<Type>)>,
}

#[derive(Debug, Clone)]
pub struct StructInfo {
    pub fields: HashMap<String, Type>,
}

pub struct EnumInfo {
    pub variants: Vec<String>,
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut structs = HashMap::new();
        let mut http_response_fields = HashMap::new();
        http_response_fields.insert("status_code".to_string(), Type::Int);
        http_response_fields.insert("status_text".to_string(), Type::String);
        http_response_fields.insert("headers".to_string(), Type::Array(Box::new(Type::String)));
        http_response_fields.insert("body".to_string(), Type::String);
        structs.insert(
            "HttpResponse".to_string(),
            StructInfo {
                fields: http_response_fields,
            },
        );

        TypeChecker {
            variables: HashMap::new(),
            module_bindings: HashMap::new(),
            functions: HashMap::new(),
            structs,
            struct_methods: HashMap::new(),
            enums: HashMap::new(),
            modules: HashMap::new(),
            current_function_return_type: None,
        }
    }

    fn check_expression(&mut self, expr: &Expression) -> Result<Type, String> {
        self.check_expression_with_expected_type(expr, None)
    }

    fn check_expression_with_expected_type(
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

            Expression::BinaryOp(left, op, right) => {
                let left_type = self.check_expression(left)?;
                let right_type = self.check_expression(right)?;

                match op {
                    Operator::Add
                    | Operator::Subtract
                    | Operator::Multiply
                    | Operator::Divide
                    | Operator::Modulo => {
                        if left_type == Type::Int && right_type == Type::Int {
                            Ok(Type::Int)
                        } else if (left_type == Type::Float || left_type == Type::Int)
                            && (right_type == Type::Float || right_type == Type::Int)
                        {
                            Ok(Type::Float)
                        } else if left_type == Type::String || right_type == Type::String {
                            Ok(Type::String)
                        } else {
                            Err(format!(
                                "Type mismatch in arithmetic operation: {:?} {:?} {:?}",
                                left_type, op, right_type
                            ))
                        }
                    }
                    Operator::UnaryMinus | Operator::Not => {
                        Err(format!("Unary operator {:?} used in binary context", op))
                    }

                    Operator::Equal
                    | Operator::NotEqual
                    | Operator::LessThan
                    | Operator::GreaterThan
                    | Operator::LessEqual
                    | Operator::GreaterEqual => {
                        if left_type != right_type {
                            return Err(format!(
                                "Type mismatch in comparison: {:?} vs {:?}",
                                left_type, right_type
                            ));
                        }
                        Ok(Type::Bool)
                    }

                    Operator::And | Operator::Or => {
                        if left_type != Type::Bool || right_type != Type::Bool {
                            return Err(format!(
                                "Logical operators require boolean operands, got {:?} and {:?}",
                                left_type, right_type
                            ));
                        }
                        Ok(Type::Bool)
                    }
                }
            }

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

    fn load_module_for_type_checking(&mut self, module_name: &str) -> Result<(), String> {
        if self.modules.contains_key(module_name) {
            return Ok(());
        }

        let module_path = crate::paths::resolve_module_path(module_name);

        let content = fs::read_to_string(&module_path)
            .map_err(|e| format!("Failed to load module '{}': {}", module_path, e))?;

        let lexer = crate::lexer::Lexer::new(content.clone());
        let mut parser = crate::parser::Parser::new(lexer, content);
        let ast = parser.parse().map_err(|e| {
            format!(
                "Failed to parse module '{}': {}",
                module_path,
                e.with_filename(module_path.clone()).format()
            )
        })?;

        let mut module_checker = TypeChecker::new();

        module_checker.check(&ast)?;

        let module_info = ModuleInfo {
            variables: module_checker.variables,
            functions: module_checker.functions.clone(),
        };

        for (name, (return_type, param_types)) in &module_checker.functions {
            self.functions
                .insert(name.clone(), (return_type.clone(), param_types.clone()));
        }

        for (name, struct_info) in &module_checker.structs {
            self.structs.insert(name.clone(), struct_info.clone());
        }

        for (struct_name, methods) in &module_checker.struct_methods {
            for (method_name, (return_type, param_types)) in methods {
                self.struct_methods
                    .entry(struct_name.clone())
                    .or_default()
                    .insert(
                        method_name.clone(),
                        (return_type.clone(), param_types.clone()),
                    );
            }
        }

        self.modules.insert(module_name.to_string(), module_info);

        for (nested_name, nested_info) in module_checker.modules {
            self.modules.entry(nested_name).or_insert(nested_info);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ASTNode, Expression};
    use std::collections::HashMap;

    #[test]
    fn test_uninitialized_struct_declaration() {
        let mut checker = TypeChecker::new();
        let mut fields = HashMap::new();
        fields.insert("x".to_string(), Type::Int);
        fields.insert("y".to_string(), Type::Int);
        checker
            .structs
            .insert("Point".to_string(), StructInfo { fields });

        let node = ASTNode::VariableDeclTyped(
            "p".to_string(),
            "Point".to_string(),
            Box::new(Expression::Uninitialized),
        );
        assert!(checker.check(&node).is_ok());
    }

    #[test]
    fn test_uninitialized_only_for_struct_type() {
        let mut checker = TypeChecker::new();
        let node = ASTNode::VariableDeclTyped(
            "x".to_string(),
            "int".to_string(),
            Box::new(Expression::Uninitialized),
        );
        assert!(checker.check(&node).is_err());
    }

    #[test]
    fn test_variable_declaration() {
        let mut checker = TypeChecker::new();
        let node = ASTNode::VariableDeclTyped(
            "x".to_string(),
            "int".to_string(),
            Box::new(Expression::Integer(5)),
        );

        assert!(checker.check(&node).is_ok());
    }

    #[test]
    fn test_type_mismatch() {
        let mut checker = TypeChecker::new();
        let node = ASTNode::VariableDeclTyped(
            "x".to_string(),
            "int".to_string(),
            Box::new(Expression::StringLiteral("hello".to_string())),
        );

        assert!(checker.check(&node).is_err());
    }

    #[test]
    fn test_undeclared_variable() {
        let mut checker = TypeChecker::new();
        let expr = Expression::Identifier("x".to_string());

        assert!(checker.check_expression(&expr).is_err());
    }

    #[test]
    fn test_empty_nested_array_literal_with_expected_matrix_type() {
        let mut checker = TypeChecker::new();
        let node = ASTNode::VariableDeclTyped(
            "m".to_string(),
            "int[][]".to_string(),
            Box::new(Expression::ArrayLiteral(vec![
                Expression::ArrayLiteral(vec![]),
                Expression::ArrayLiteral(vec![]),
            ])),
        );
        assert!(checker.check(&node).is_ok());
    }

    #[test]
    fn test_module_method_call_resolves_return_type() {
        let mut checker = TypeChecker::new();
        checker.variables.insert("json".to_string(), Type::Module);
        checker
            .module_bindings
            .insert("json".to_string(), "json".to_string());

        let mut module_functions = HashMap::new();
        module_functions.insert("load".to_string(), (Type::String, vec![Type::String]));
        checker.modules.insert(
            "json".to_string(),
            ModuleInfo {
                variables: HashMap::new(),
                functions: module_functions,
            },
        );

        let expr = Expression::MethodCall(
            Box::new(Expression::Identifier("json".to_string())),
            "load".to_string(),
            vec![Expression::StringLiteral("test.json".to_string())],
        );

        assert_eq!(checker.check_expression(&expr).unwrap(), Type::String);
    }

    #[test]
    fn tcp_builtins_typecheck_like_network_wrappers() {
        let src = r#"
export fun listen(addr: string, backlog: int) -> TcpListener {
    return tcp_listen(addr, backlog);
}

export fun accept(listener: TcpListener) -> TcpStream {
    return tcp_accept(listener);
}

export fun close_listener(listener: TcpListener) -> void {
    tcp_close_listener(listener);
}
"#;
        let lexer = crate::lexer::Lexer::new(src.to_string());
        let mut parser = crate::parser::Parser::new(lexer, src.to_string());
        let ast = parser.parse().expect("parse tcp wrapper module");
        let mut checker = TypeChecker::new();
        checker
            .check(&ast)
            .expect("typecheck tcp builtins in functions");
    }
}
