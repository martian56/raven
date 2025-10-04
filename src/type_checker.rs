use crate::ast::{ASTNode, Expression, Operator};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    Void,
    Array(Box<Type>), // Add array type support
    Struct(String), // Struct type
    Enum(String), // Enum type
    Module, // Module type
    Unknown,
}

impl Type {
    pub fn from_string(s: &str) -> Type {
        match s {
            "int" => Type::Int,
            "float" => Type::Float,
            "bool" => Type::Bool,
            "string" => Type::String,
            "void" => Type::Void,
            "int[]" => Type::Array(Box::new(Type::Int)),
            "float[]" => Type::Array(Box::new(Type::Float)),
            "bool[]" => Type::Array(Box::new(Type::Bool)),
            "String[]" => Type::Array(Box::new(Type::String)),
            _ => Type::Struct(s.to_string()), // Custom type (struct or enum - will be resolved later)
        }
    }
    
    pub fn from_string_with_context(s: &str, enums: &HashMap<String, EnumInfo>, structs: &HashMap<String, StructInfo>) -> Type {
        match s {
            "int" => Type::Int,
            "float" => Type::Float,
            "bool" => Type::Bool,
            "string" => Type::String,
            "void" => Type::Void,
            "int[]" => Type::Array(Box::new(Type::Int)),
            "float[]" => Type::Array(Box::new(Type::Float)),
            "bool[]" => Type::Array(Box::new(Type::Bool)),
            "String[]" => Type::Array(Box::new(Type::String)),
            _ => {
                // Check if it's an enum first, then struct
                if enums.contains_key(s) {
                    Type::Enum(s.to_string())
                } else if structs.contains_key(s) {
                    Type::Struct(s.to_string())
                } else {
                    Type::Struct(s.to_string()) // Default to struct for backward compatibility
                }
            }
        }
    }
}

pub struct TypeChecker {
    // Symbol table: variable_name -> type
    variables: HashMap<String, Type>,
    // Function table: function_name -> (return_type, param_types)
    functions: HashMap<String, (Type, Vec<Type>)>,
    // Struct table: struct_name -> StructInfo
    structs: HashMap<String, StructInfo>,
    // Enum table: enum_name -> EnumInfo
    enums: HashMap<String, EnumInfo>,
    // Module table: module_name -> ModuleInfo
    modules: HashMap<String, ModuleInfo>,
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

impl TypeChecker {
    pub fn new() -> Self {
        TypeChecker {
            variables: HashMap::new(),
            functions: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            modules: HashMap::new(),
        }
    }

    pub fn check(&mut self, node: &ASTNode) -> Result<Type, String> {
        match node {
            ASTNode::VariableDecl(name, expr) => {
                let expr_type = self.check_expression(expr)?;
                self.variables.insert(name.clone(), expr_type.clone());
                Ok(Type::Void)
            }

            ASTNode::VariableDeclTyped(name, type_str, expr) => {
                let declared_type = Type::from_string_with_context(type_str, &self.enums, &self.structs);
                let expr_type = self.check_expression_with_expected_type(expr, Some(&declared_type))?;

                if declared_type != expr_type {
                    return Err(format!(
                        "Type mismatch in variable '{}': expected {:?}, got {:?}",
                        name, declared_type, expr_type
                    ));
                }

                self.variables.insert(name.clone(), declared_type);
                Ok(Type::Void)
            }

            ASTNode::Assignment(target, expr) => {
                let expr_type = self.check_expression(expr)?;
                
                // Check the assignment target
                match target.as_ref() {
                    Expression::Identifier(name) => {
                        // Simple variable assignment
                        if let Some(var_type) = self.variables.get(name) {
                            if var_type != &expr_type {
                                return Err(format!(
                                    "Type mismatch in assignment to '{}': expected {:?}, got {:?}",
                                    name, var_type, expr_type
                                ));
                            }
                            Ok(Type::Void)
                        } else {
                            Err(format!("Variable '{}' not declared", name))
                        }
                    }
                    Expression::FieldAccess(object, _field_name) => {
                        // Field access assignment: object.field = value
                        let _object_type = self.check_expression(object)?;
                        
                        // For now, we'll allow field assignments without strict type checking
                        // This is a simplified implementation
                        Ok(Type::Void)
                    }
                    Expression::ArrayIndex(array_expr, index_expr) => {
                        // Array indexing assignment: array[index] = value
                        let _array_type = self.check_expression(array_expr)?;
                        let index_type = self.check_expression(index_expr)?;
                        
                        // Check that index is an integer
                        if index_type != Type::Int {
                            return Err(format!(
                                "Array index must be an integer, got {:?}",
                                index_type
                            ));
                        }
                        
                        // For now, we'll allow array assignments without strict type checking
                        // This is a simplified implementation
                        Ok(Type::Void)
                    }
                    _ => {
                        // Other complex assignment targets
                        // For now, we'll allow them without strict type checking
                        Ok(Type::Void)
                    }
                }
            }

            ASTNode::FunctionDecl(name, return_type_str, params, body) => {
                let return_type = Type::from_string_with_context(return_type_str, &self.enums, &self.structs);
                
                // Store parameter types in local scope
                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| Type::from_string_with_context(&p.param_type, &self.enums, &self.structs))
                    .collect();

                // Add parameters to variables table
                for (i, param) in params.iter().enumerate() {
                    self.variables.insert(param.name.clone(), param_types[i].clone());
                }

                // Register the function
                self.functions.insert(name.clone(), (return_type.clone(), param_types));

                // Check function body
                self.check(body)?;

                Ok(Type::Void)
            }

            ASTNode::StructDecl(name, fields) => {
                let mut struct_info = StructInfo {
                    fields: HashMap::new(),
                };
                
                // Process each field
                for field in fields {
                    let field_type = Type::from_string_with_context(&field.field_type, &self.enums, &self.structs);
                    struct_info.fields.insert(field.name.clone(), field_type);
                }
                
                // Register the struct
                self.structs.insert(name.clone(), struct_info);
                
                Ok(Type::Void)
            }

            ASTNode::EnumDecl(name, variants) => {
                // Register the enum with its variants
                let enum_info = EnumInfo {
                    variants: variants.clone(),
                };
                
                self.enums.insert(name.clone(), enum_info);
                
                Ok(Type::Void)
            }

            ASTNode::IfStatement(condition, then_block, else_if, else_block) => {
                let cond_type = self.check_expression(condition)?;
                if cond_type != Type::Bool {
                    return Err(format!(
                        "Condition in if statement must be boolean, got {:?}",
                        cond_type
                    ));
                }

                self.check(then_block)?;

                if let Some(else_if_node) = else_if {
                    self.check(else_if_node)?;
                }

                if let Some(else_node) = else_block {
                    self.check(else_node)?;
                }

                Ok(Type::Void)
            }

            ASTNode::WhileLoop(condition, body) => {
                let cond_type = self.check_expression(condition)?;
                if cond_type != Type::Bool {
                    return Err(format!(
                        "Condition in while loop must be boolean, got {:?}",
                        cond_type
                    ));
                }

                self.check(body)?;
                Ok(Type::Void)
            }

            ASTNode::ForLoop(init, condition, increment, body) => {
                self.check(init)?;

                let cond_type = self.check_expression(condition)?;
                if cond_type != Type::Bool {
                    return Err(format!(
                        "Condition in for loop must be boolean, got {:?}",
                        cond_type
                    ));
                }

                self.check(increment)?;
                self.check(body)?;

                Ok(Type::Void)
            }

            ASTNode::Block(statements) => {
                for stmt in statements {
                    self.check(stmt)?;
                }
                Ok(Type::Void)
            }

            ASTNode::Print(expr) => {
                self.check_expression(expr)?;
                Ok(Type::Void)
            }

            ASTNode::Return(expr) => {
                self.check_expression(expr)?;
                Ok(Type::Void)
            }

            ASTNode::FunctionCall(name, args) => {
                // Check function exists and validate arguments
                self.check_expression(&Expression::FunctionCall(name.clone(), args.clone()))?;
                Ok(Type::Void)
            }
            
            ASTNode::MethodCall(object, method_name, args) => {
                // Check method call and validate arguments
                self.check_expression(&Expression::MethodCall(object.clone(), method_name.clone(), args.clone()))?;
                Ok(Type::Void)
            }
            
            ASTNode::ExpressionStatement(expr) => {
                // Type check standalone expressions
                self.check_expression(expr)?;
                Ok(Type::Void)
            }
            
            ASTNode::Import(module_name, alias) => {
                // Load the module during type checking
                self.load_module_for_type_checking(module_name)?;
                
                // If there's an alias, add it to variables
                if let Some(alias_name) = alias {
                    self.variables.insert(alias_name.clone(), Type::Module);
                }
                Ok(Type::Void)
            }
            
            ASTNode::ImportSelective(module_name, items) => {
                // Load the module during type checking
                self.load_module_for_type_checking(module_name)?;
                
                // Import specific items from the module
                if let Some(module) = self.modules.get(module_name) {
                    for item in items {
                        if let Some(var_type) = module.variables.get(item) {
                            self.variables.insert(item.clone(), var_type.clone());
                        } else if let Some((return_type, param_types)) = module.functions.get(item) {
                            // Import the function with its parameter types
                            self.functions.insert(item.clone(), (return_type.clone(), param_types.clone()));
                        } else {
                            return Err(format!("Item '{}' not found in module '{}'", item, module_name));
                        }
                    }
                } else {
                    return Err(format!("Module '{}' not found", module_name));
                }
                Ok(Type::Void)
            }
            
            ASTNode::Export(stmt) => {
                // Check the exported statement
                self.check(stmt)?;
                Ok(Type::Void)
            }
        }
    }

    fn check_expression(&mut self, expr: &Expression) -> Result<Type, String> {
        self.check_expression_with_expected_type(expr, None)
    }
    
    fn check_expression_with_expected_type(&mut self, expr: &Expression, expected_type: Option<&Type>) -> Result<Type, String> {
        match expr {
            Expression::Integer(_) => Ok(Type::Int),
            Expression::Float(_) => Ok(Type::Float),
            Expression::Boolean(_) => Ok(Type::Bool),
            Expression::StringLiteral(_) => Ok(Type::String),

            Expression::Identifier(name) => {
                if let Some(var_type) = self.variables.get(name) {
                    Ok(var_type.clone())
                } else {
                    Err(format!("Variable '{}' not declared", name))
                }
            }

            Expression::UnaryOp(op, expr) => {
                let expr_type = self.check_expression(expr)?;
                
                match op {
                    Operator::UnaryMinus => {
                        match expr_type {
                            Type::Int | Type::Float => Ok(expr_type),
                            _ => Err(format!("Cannot apply unary minus to {:?}", expr_type)),
                        }
                    }
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
                    Operator::Add | Operator::Subtract | Operator::Multiply | Operator::Divide | Operator::Modulo => {
                        if left_type == Type::Int && right_type == Type::Int {
                            Ok(Type::Int)
                        } else if (left_type == Type::Float || left_type == Type::Int)
                            && (right_type == Type::Float || right_type == Type::Int)
                        {
                            Ok(Type::Float)
                        } else if left_type == Type::String && right_type == Type::String {
                            Ok(Type::String) // String concatenation
                        } else if left_type == Type::String || right_type == Type::String {
                            Ok(Type::String) // String + number or number + string
                        } else {
                            Err(format!(
                                "Type mismatch in arithmetic operation: {:?} {:?} {:?}",
                                left_type, op, right_type
                            ))
                        }
                    }
                    Operator::UnaryMinus | Operator::Not => {
                        // These should not appear in binary operations
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
                // Check if this is a built-in function first
                if let Some(return_type) = self.check_builtin_function(name, args)? {
                    return Ok(return_type);
                }
                
                // Otherwise, check regular function
                // Look up the function and clone to avoid borrow issues
                if let Some((return_type, param_types)) = self.functions.get(name).cloned() {
                    // Check argument count
                    if args.len() != param_types.len() {
                        return Err(format!(
                            "Function '{}' expects {} arguments, got {}",
                            name,
                            param_types.len(),
                            args.len()
                        ));
                    }

                    // Check argument types
                    for (i, arg) in args.iter().enumerate() {
                        let arg_type = self.check_expression(arg)?;
                        if arg_type != param_types[i] {
                            return Err(format!(
                                "Function '{}' parameter {} expects {:?}, got {:?}",
                                name,
                                i + 1,
                                param_types[i],
                                arg_type
                            ));
                        }
                    }

                    Ok(return_type)
                } else {
                    Err(format!("Function '{}' not declared", name))
                }
            }

            Expression::ArrayLiteral(elements) => {
                if elements.is_empty() {
                    // Empty array - use expected type if available
                    if let Some(expected) = expected_type {
                        if let Type::Array(element_type) = expected {
                            return Ok(Type::Array(element_type.clone()));
                        }
                    }
                    return Err("Cannot infer type of empty array".to_string());
                }
                
                // Check that all elements have the same type
                let first_type = self.check_expression_with_expected_type(&elements[0], None)?;
                for element in elements.iter().skip(1) {
                    let element_type = self.check_expression_with_expected_type(element, None)?;
                    if element_type != first_type {
                        return Err(format!(
                            "Array elements must have the same type, got {:?} and {:?}",
                            first_type, element_type
                        ));
                    }
                }
                
                // Return array type
                Ok(Type::Array(Box::new(first_type)))
            }

            Expression::ArrayIndex(array_expr, index_expr) => {
                let index_type = self.check_expression(index_expr)?;
                if index_type != Type::Int {
                    return Err(format!(
                        "Array index must be integer, got {:?}",
                        index_type
                    ));
                }
                
                let array_type = self.check_expression(array_expr)?;
                match array_type {
                    Type::Array(element_type) => Ok(*element_type),
                    Type::String => Ok(Type::String), // String indexing returns String (single character)
                    _ => Err("Cannot index non-array or non-string value".to_string()),
                }
            }
            
            Expression::MethodCall(object_expr, method_name, args) => {
                let object_type = self.check_expression(object_expr)?;
                
                // Check array methods
                if let Type::Array(element_type) = object_type {
                    match method_name.as_str() {
                        "push" => {
                            if args.len() != 1 {
                                return Err(format!("push() expects 1 argument, got {}", args.len()));
                            }
                            let arg_type = self.check_expression(&args[0])?;
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
                            Ok(*element_type) // pop() returns the element type
                        }
                        "slice" => {
                            if args.len() != 2 {
                                return Err(format!("slice() expects 2 arguments, got {}", args.len()));
                            }
                            let start_type = self.check_expression(&args[0])?;
                            let end_type = self.check_expression(&args[1])?;
                            if start_type != Type::Int || end_type != Type::Int {
                                return Err("slice() arguments must be integers".to_string());
                            }
                            Ok(Type::Array(element_type)) // slice() returns array of same type
                        }
                        "join" => {
                            if args.len() != 1 {
                                return Err(format!("join() expects 1 argument, got {}", args.len()));
                            }
                            let delimiter_type = self.check_expression(&args[0])?;
                            if delimiter_type != Type::String {
                                return Err("join() delimiter must be string".to_string());
                            }
                            Ok(Type::String) // join() returns string
                        }
                        _ => Err(format!("Unknown method '{}' for array", method_name)),
                    }
                } else if let Type::Module = object_type {
                    // Handle module method calls
                    // For now, we'll assume module methods can return any type
                    // TODO: Implement proper module method type checking
                    Ok(Type::Unknown)
                } else if let Type::String = object_type {
                    // Handle string method calls
                    match method_name.as_str() {
                        "slice" => {
                            if args.len() != 2 {
                                return Err(format!("slice() expects 2 arguments, got {}", args.len()));
                            }
                            let start_type = self.check_expression(&args[0])?;
                            let end_type = self.check_expression(&args[1])?;
                            if start_type != Type::Int || end_type != Type::Int {
                                return Err("slice() arguments must be integers".to_string());
                            }
                            Ok(Type::String) // slice() returns string
                        }
                        "split" => {
                            if args.len() != 1 {
                                return Err(format!("split() expects 1 argument, got {}", args.len()));
                            }
                            let delimiter_type = self.check_expression(&args[0])?;
                            if delimiter_type != Type::String {
                                return Err("split() delimiter must be string".to_string());
                            }
                            Ok(Type::Array(Box::new(Type::String))) // split() returns array of strings
                        }
                        "replace" => {
                            if args.len() != 2 {
                                return Err(format!("replace() expects 2 arguments, got {}", args.len()));
                            }
                            let from_type = self.check_expression(&args[0])?;
                            let to_type = self.check_expression(&args[1])?;
                            if from_type != Type::String || to_type != Type::String {
                                return Err("replace() arguments must be strings".to_string());
                            }
                            Ok(Type::String) // replace() returns string
                        }
                        _ => Err(format!("Unknown method '{}' for string", method_name)),
                    }
                } else {
                    Err(format!("Cannot call methods on non-array, non-module, or non-string value of type {:?}", object_type))
                }
            }
            
            Expression::StructInstantiation(struct_name, fields) => {
                // Check if struct is defined
                if let Some(struct_info) = self.structs.get(struct_name) {
                    // Clone the struct info to avoid borrowing conflicts
                    let struct_info_clone = struct_info.clone();
                    
                    // Check that all fields are provided and have correct types
                    for (field_name, field_value) in fields {
                        if let Some(expected_type) = struct_info_clone.fields.get(field_name) {
                            let actual_type = self.check_expression_with_expected_type(field_value, Some(expected_type))?;
                            if actual_type != *expected_type {
                                return Err(format!(
                                    "Field '{}' in struct '{}' expects {:?}, got {:?}",
                                    field_name, struct_name, expected_type, actual_type
                                ));
                            }
                        } else {
                            return Err(format!(
                                "Field '{}' not found in struct '{}'",
                                field_name, struct_name
                            ));
                        }
                    }
                    
                    // Check that all required fields are provided
                    for (field_name, _) in &struct_info_clone.fields {
                        if !fields.iter().any(|(name, _)| name == field_name) {
                            return Err(format!(
                                "Missing required field '{}' in struct '{}'",
                                field_name, struct_name
                            ));
                        }
                    }
                    
                    Ok(Type::Struct(struct_name.clone()))
                } else {
                    Err(format!("Struct '{}' not declared", struct_name))
                }
            }
            
            Expression::FieldAccess(object_expr, field_name) => {
                let object_type = self.check_expression(object_expr)?;
                
                if let Type::Struct(struct_name) = object_type {
                    if let Some(struct_info) = self.structs.get(&struct_name) {
                        if let Some(field_type) = struct_info.fields.get(field_name) {
                            Ok(field_type.clone())
                        } else {
                            Err(format!(
                                "Field '{}' not found in struct '{}'",
                                field_name, struct_name
                            ))
                        }
                    } else {
                        Err(format!("Struct '{}' not found", struct_name))
                    }
                } else {
                    Err(format!("Cannot access field on non-struct value of type {:?}", object_type))
                }
            }

            Expression::EnumVariant(enum_name, variant_name) => {
                // Check if the enum exists
                if let Some(enum_info) = self.enums.get(enum_name) {
                    // Check if the variant exists in this enum
                    if enum_info.variants.contains(variant_name) {
                        Ok(Type::Enum(enum_name.clone()))
                    } else {
                        Err(format!(
                            "Variant '{}' not found in enum '{}'", 
                            variant_name, enum_name
                        ))
                    }
                } else {
                    Err(format!("Enum '{}' not found", enum_name))
                }
            }
        }
    }

    fn check_builtin_function(&mut self, name: &str, args: &[Expression]) -> Result<Option<Type>, String> {
        match name {
            "len" => {
                if args.len() != 1 {
                    return Err(format!("len() expects 1 argument, got {}", args.len()));
                }
                
                let arg_type = self.check_expression(&args[0])?;
                match arg_type {
                    Type::Array(_) | Type::String => Ok(Some(Type::Int)),
                    _ => Err(format!("len() expects array or string, got {:?}", arg_type)),
                }
            }
            
            "type" => {
                if args.len() != 1 {
                    return Err(format!("type() expects 1 argument, got {}", args.len()));
                }
                
                // type() can accept any type and always returns string
                self.check_expression(&args[0])?;
                Ok(Some(Type::String))
            }
            
            "print" => {
                if args.is_empty() {
                    return Err("print() expects at least 1 argument".to_string());
                }
                
                // Check all arguments are valid expressions
                for arg in args {
                    self.check_expression(arg)?;
                }
                
                // print() always returns void
                Ok(Some(Type::Void))
            }
            
            "input" => {
                if args.len() > 1 {
                    return Err(format!("input() expects 0 or 1 argument, got {}", args.len()));
                }
                
                // Check prompt argument if provided
                if args.len() == 1 {
                    let prompt_type = self.check_expression(&args[0])?;
                    if prompt_type != Type::String {
                        return Err("input() prompt must be a string".to_string());
                    }
                }
                
                // input() always returns string
                Ok(Some(Type::String))
            }
            
            "read_file" => {
                if args.len() != 1 {
                    return Err(format!("read_file() expects 1 argument, got {}", args.len()));
                }
                
                let filename_type = self.check_expression(&args[0])?;
                if filename_type != Type::String {
                    return Err("read_file() filename must be a string".to_string());
                }
                
                // read_file() always returns string
                Ok(Some(Type::String))
            }
            
            "write_file" => {
                if args.len() != 2 {
                    return Err(format!("write_file() expects 2 arguments, got {}", args.len()));
                }
                
                let filename_type = self.check_expression(&args[0])?;
                if filename_type != Type::String {
                    return Err("write_file() filename must be a string".to_string());
                }
                
                // Content can be any type (will be converted to string)
                self.check_expression(&args[1])?;
                
                // write_file() always returns void
                Ok(Some(Type::Void))
            }
            
            "append_file" => {
                if args.len() != 2 {
                    return Err(format!("append_file() expects 2 arguments, got {}", args.len()));
                }
                
                let filename_type = self.check_expression(&args[0])?;
                if filename_type != Type::String {
                    return Err("append_file() filename must be a string".to_string());
                }
                
                // Content can be any type (will be converted to string)
                self.check_expression(&args[1])?;
                
                // append_file() always returns void
                Ok(Some(Type::Void))
            }
            
            "file_exists" => {
                if args.len() != 1 {
                    return Err(format!("file_exists() expects 1 argument, got {}", args.len()));
                }
                
                let filename_type = self.check_expression(&args[0])?;
                if filename_type != Type::String {
                    return Err("file_exists() filename must be a string".to_string());
                }
                
                // file_exists() always returns bool
                Ok(Some(Type::Bool))
            }
            
            "format" => {
                if args.len() < 1 {
                    return Err(format!("format() expects at least 1 argument, got {}", args.len()));
                }
                
                let template_type = self.check_expression(&args[0])?;
                if template_type != Type::String {
                    return Err("format() template must be a string".to_string());
                }
                
                // format() always returns string
                Ok(Some(Type::String))
            }
            
            "enum_from_string" => {
                if args.len() != 2 {
                    return Err(format!("enum_from_string() expects 2 arguments, got {}", args.len()));
                }
                
                let enum_name_type = self.check_expression(&args[0])?;
                let variant_name_type = self.check_expression(&args[1])?;
                
                if enum_name_type != Type::String {
                    return Err("enum_from_string() first argument must be a string".to_string());
                }
                
                if variant_name_type != Type::String {
                    return Err("enum_from_string() second argument must be a string".to_string());
                }
                
                // Try to determine the enum type from the first argument if it's a string literal
                if let Expression::StringLiteral(enum_name) = &args[0] {
                    if self.enums.contains_key(enum_name) {
                        return Ok(Some(Type::Enum(enum_name.clone())));
                    }
                }
                
                // If we can't determine the specific enum type, return a generic enum
                // This allows the function to work with dynamic string values
                Ok(Some(Type::Enum("Unknown".to_string())))
            }
            
            _ => Ok(None), // Not a built-in function
        }
    }
    
    fn load_module_for_type_checking(&mut self, module_name: &str) -> Result<(), String> {
        // Check if module is already loaded
        if self.modules.contains_key(module_name) {
            return Ok(());
        }
        
        // Load module file
        let module_path = if module_name.ends_with(".rv") {
            module_name.to_string()
        } else {
            // First try in lib/ directory for standard library modules
            let lib_path = format!("lib/{}.rv", module_name);
            if std::path::Path::new(&lib_path).exists() {
                lib_path
            } else {
                format!("{}.rv", module_name)
            }
        };
        
        let content = fs::read_to_string(&module_path)
            .map_err(|e| format!("Failed to load module '{}': {}", module_path, e))?;
        
        // Parse the module
        let lexer = crate::lexer::Lexer::new(content.clone());
        let mut parser = crate::parser::Parser::new(lexer, content);
        let ast = parser.parse()
            .map_err(|e| format!("Failed to parse module '{}': {}", module_path, e.format()))?;
        
        // Create a new type checker for the module
        let mut module_checker = TypeChecker::new();
        
        // Analyze the module to extract type information
        module_checker.check(&ast)?;
        
        // Extract module information
        let module_info = ModuleInfo {
            variables: module_checker.variables,
            functions: module_checker.functions.clone(),
        };
        
        // Merge functions from the module into the global scope
        for (name, (return_type, param_types)) in &module_checker.functions {
            self.functions.insert(name.clone(), (return_type.clone(), param_types.clone()));
        }
        
        // Merge structs from the module into the global scope
        for (name, struct_info) in &module_checker.structs {
            self.structs.insert(name.clone(), struct_info.clone());
        }
        
        // Store the module
        self.modules.insert(module_name.to_string(), module_info);
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expression, ASTNode};

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
}

