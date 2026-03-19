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
    Array(Box<Type>),
    Struct(String),
    Enum(String),
    Module,
    Unknown,
}

impl Type {
    pub fn fmt_for_user(&self) -> String {
        match self {
            Type::Int => "int".to_string(),
            Type::Float => "float".to_string(),
            Type::Bool => "bool".to_string(),
            Type::String => "string".to_string(),
            Type::Void => "void".to_string(),
            Type::Array(inner) => format!("{}[]", inner.fmt_for_user()),
            Type::Struct(s) => s.clone(),
            Type::Enum(s) => s.clone(),
            Type::Module => "module".to_string(),
            Type::Unknown => "unknown".to_string(),
        }
    }

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
            _ => Type::Struct(s.to_string()),
        }
    }

    pub fn from_string_with_context(
        s: &str,
        enums: &HashMap<String, EnumInfo>,
        _structs: &HashMap<String, StructInfo>,
    ) -> Type {
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
                if enums.contains_key(s) {
                    Type::Enum(s.to_string())
                } else {
                    Type::Struct(s.to_string())
                }
            }
        }
    }
}

pub struct TypeChecker {
    variables: HashMap<String, Type>,
    functions: HashMap<String, (Type, Vec<Type>)>,
    structs: HashMap<String, StructInfo>,
    struct_methods: HashMap<String, HashMap<String, (Type, Vec<Type>)>>,
    enums: HashMap<String, EnumInfo>,
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

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeChecker {
    pub fn new() -> Self {
        TypeChecker {
            variables: HashMap::new(),
            functions: HashMap::new(),
            structs: HashMap::new(),
            struct_methods: HashMap::new(),
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
                let declared_type =
                    Type::from_string_with_context(type_str, &self.enums, &self.structs);
                let expr_type =
                    self.check_expression_with_expected_type(expr, Some(&declared_type))?;

                if declared_type != expr_type {
                    let expected = declared_type.fmt_for_user();
                    let got = expr_type.fmt_for_user();
                    return Err(format!(
                        "Type mismatch in variable '{}': expected {}, got {}\n   = help: Change the expression to match type '{}', or change the variable's declared type to '{}'.",
                        name, expected, got, expected, got
                    ));
                }

                self.variables.insert(name.clone(), declared_type);
                Ok(Type::Void)
            }

            ASTNode::Assignment(target, expr) => {
                let expr_type = self.check_expression(expr)?;

                match target.as_ref() {
                    Expression::Identifier(name) => {
                        if let Some(var_type) = self.variables.get(name) {
                            if var_type != &expr_type {
                                let expected = var_type.fmt_for_user();
                                let got = expr_type.fmt_for_user();
                                return Err(format!(
                                    "Type mismatch in assignment to '{}': expected {}, got {}\n   = help: The value must match the variable's type '{}'. Consider converting or changing the expression.",
                                    name, expected, got, expected
                                ));
                            }
                            Ok(Type::Void)
                        } else {
                            Err(format!(
                                "Variable '{}' not declared\n   = help: Declare the variable with 'let {}: type = value;' before using it.",
                                name, name
                            ))
                        }
                    }
                    Expression::FieldAccess(object, _field_name) => {
                        let _object_type = self.check_expression(object)?;

                        Ok(Type::Void)
                    }
                    Expression::ArrayIndex(array_expr, index_expr) => {
                        let _array_type = self.check_expression(array_expr)?;
                        let index_type = self.check_expression(index_expr)?;

                        if index_type != Type::Int {
                            return Err(format!(
                                "Array index must be an integer, got {}\n   = help: Use an int variable or expression, e.g. arr[i] or arr[i + 1] where i is int.",
                                index_type.fmt_for_user()
                            ));
                        }

                        Ok(Type::Void)
                    }
                    _ => Ok(Type::Void),
                }
            }

            ASTNode::FunctionDecl(name, return_type_str, params, body) => {
                let return_type =
                    Type::from_string_with_context(return_type_str, &self.enums, &self.structs);

                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| {
                        Type::from_string_with_context(&p.param_type, &self.enums, &self.structs)
                    })
                    .collect();

                for (i, param) in params.iter().enumerate() {
                    self.variables
                        .insert(param.name.clone(), param_types[i].clone());
                }

                self.functions
                    .insert(name.clone(), (return_type.clone(), param_types));

                self.check(body)?;

                Ok(Type::Void)
            }

            ASTNode::StructDecl(name, fields) => {
                let mut struct_info = StructInfo {
                    fields: HashMap::new(),
                };

                for field in fields {
                    let field_type = Type::from_string_with_context(
                        &field.field_type,
                        &self.enums,
                        &self.structs,
                    );
                    struct_info.fields.insert(field.name.clone(), field_type);
                }

                self.structs.insert(name.clone(), struct_info);

                Ok(Type::Void)
            }

            ASTNode::ImplBlock(struct_name, methods) => {
                if !self.structs.contains_key(struct_name) {
                    return Err(format!(
                        "Cannot impl for '{}': struct not found\n   = help: Define the struct first with 'struct {} {{ ... }}'",
                        struct_name, struct_name
                    ));
                }

                for (method_name, return_type_str, params, body) in methods {
                    let return_type =
                        Type::from_string_with_context(return_type_str, &self.enums, &self.structs);
                    let param_types: Vec<Type> = params
                        .iter()
                        .map(|p| {
                            Type::from_string_with_context(
                                &p.param_type,
                                &self.enums,
                                &self.structs,
                            )
                        })
                        .collect();

                    self.struct_methods
                        .entry(struct_name.clone())
                        .or_default()
                        .insert(
                            method_name.clone(),
                            (return_type.clone(), param_types.clone()),
                        );

                    let self_type = Type::Struct(struct_name.clone());
                    self.variables.insert("self".to_string(), self_type.clone());

                    for (i, param) in params.iter().skip(1).enumerate() {
                        if i + 1 < param_types.len() {
                            self.variables
                                .insert(param.name.clone(), param_types[i + 1].clone());
                        }
                    }

                    self.check(body)?;

                    self.variables.remove("self");
                    for param in params.iter().skip(1) {
                        self.variables.remove(&param.name);
                    }
                }

                Ok(Type::Void)
            }

            ASTNode::EnumDecl(name, variants) => {
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
                        "Condition in if statement must be boolean, got {}\n   = help: Use a comparison (e.g. x == 0, a < b) or a boolean variable. Example: if (x > 0) {{ ... }}",
                        cond_type.fmt_for_user()
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
                        "Condition in while loop must be boolean, got {}\n   = help: Use a comparison (e.g. i < 10) or boolean. Example: while (i < 10) {{ ... }}",
                        cond_type.fmt_for_user()
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
                        "Condition in for loop must be boolean, got {}\n   = help: The condition (middle part) must be a comparison. Example: for (let i: int = 0; i < 10; i = i + 1) {{ ... }}",
                        cond_type.fmt_for_user()
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
                self.check_expression(&Expression::FunctionCall(name.clone(), args.clone()))?;
                Ok(Type::Void)
            }

            ASTNode::MethodCall(object, method_name, args) => {
                self.check_expression(&Expression::MethodCall(
                    object.clone(),
                    method_name.clone(),
                    args.clone(),
                ))?;
                Ok(Type::Void)
            }

            ASTNode::ExpressionStatement(expr) => {
                self.check_expression(expr)?;
                Ok(Type::Void)
            }

            ASTNode::Import(module_name, alias) => {
                self.load_module_for_type_checking(module_name)?;

                let var_name = alias.as_ref().unwrap_or(module_name);
                self.variables.insert(var_name.clone(), Type::Module);
                Ok(Type::Void)
            }

            ASTNode::ImportSelective(module_name, items) => {
                self.load_module_for_type_checking(module_name)?;

                if let Some(module) = self.modules.get(module_name) {
                    for item in items {
                        if let Some(var_type) = module.variables.get(item) {
                            self.variables.insert(item.clone(), var_type.clone());
                        } else if let Some((return_type, param_types)) = module.functions.get(item)
                        {
                            self.functions
                                .insert(item.clone(), (return_type.clone(), param_types.clone()));
                        } else {
                            return Err(format!(
                                "Item '{}' not found in module '{}'",
                                item, module_name
                            ));
                        }
                    }
                } else {
                    return Err(format!("Module '{}' not found", module_name));
                }
                Ok(Type::Void)
            }

            ASTNode::Export(stmt) => {
                self.check(stmt)?;
                Ok(Type::Void)
            }
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
                    if let Some(Type::Array(element_type)) = expected_type {
                        return Ok(Type::Array(element_type.clone()));
                    }
                    return Err(
                        "Cannot infer type of empty array\n   = help: Give the array an explicit type, e.g. let arr: int[] = []; or add at least one element.".to_string(),
                    );
                }

                let first_type = self.check_expression_with_expected_type(&elements[0], None)?;
                for element in elements.iter().skip(1) {
                    let element_type = self.check_expression_with_expected_type(element, None)?;
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
                let object_type = self.check_expression(object_expr)?;

                if let Type::Array(element_type) = object_type {
                    match method_name.as_str() {
                        "push" => {
                            if args.len() != 1 {
                                return Err(format!(
                                    "push() expects 1 argument, got {}",
                                    args.len()
                                ));
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
                                return Err(format!(
                                    "pop() expects 0 arguments, got {}",
                                    args.len()
                                ));
                            }
                            Ok(*element_type)
                        }
                        "slice" => {
                            if args.len() != 2 {
                                return Err(format!(
                                    "slice() expects 2 arguments, got {}",
                                    args.len()
                                ));
                            }
                            let start_type = self.check_expression(&args[0])?;
                            let end_type = self.check_expression(&args[1])?;
                            if start_type != Type::Int || end_type != Type::Int {
                                return Err("slice() arguments must be integers".to_string());
                            }
                            Ok(Type::Array(element_type))
                        }
                        "join" => {
                            if args.len() != 1 {
                                return Err(format!(
                                    "join() expects 1 argument, got {}",
                                    args.len()
                                ));
                            }
                            let delimiter_type = self.check_expression(&args[0])?;
                            if delimiter_type != Type::String {
                                return Err("join() delimiter must be string".to_string());
                            }
                            Ok(Type::String)
                        }
                        _ => Err(format!("Unknown method '{}' for array", method_name)),
                    }
                } else if let Type::Module = object_type {
                    Ok(Type::Unknown)
                } else if let Type::String = object_type {
                    match method_name.as_str() {
                        "slice" => {
                            if args.len() != 2 {
                                return Err(format!(
                                    "slice() expects 2 arguments, got {}",
                                    args.len()
                                ));
                            }
                            let start_type = self.check_expression(&args[0])?;
                            let end_type = self.check_expression(&args[1])?;
                            if start_type != Type::Int || end_type != Type::Int {
                                return Err("slice() arguments must be integers".to_string());
                            }
                            Ok(Type::String)
                        }
                        "split" => {
                            if args.len() != 1 {
                                return Err(format!(
                                    "split() expects 1 argument, got {}",
                                    args.len()
                                ));
                            }
                            let delimiter_type = self.check_expression(&args[0])?;
                            if delimiter_type != Type::String {
                                return Err("split() delimiter must be string".to_string());
                            }
                            Ok(Type::Array(Box::new(Type::String)))
                        }
                        "replace" => {
                            if args.len() != 2 {
                                return Err(format!(
                                    "replace() expects 2 arguments, got {}",
                                    args.len()
                                ));
                            }
                            let from_type = self.check_expression(&args[0])?;
                            let to_type = self.check_expression(&args[1])?;
                            if from_type != Type::String || to_type != Type::String {
                                return Err("replace() arguments must be strings".to_string());
                            }
                            Ok(Type::String)
                        }
                        _ => Err(format!("Unknown method '{}' for string", method_name)),
                    }
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

    fn check_builtin_function(
        &mut self,
        name: &str,
        args: &[Expression],
    ) -> Result<Option<Type>, String> {
        match name {
            "len" => {
                if args.len() != 1 {
                    return Err(format!("len() expects 1 argument, got {}", args.len()));
                }

                let arg_type = self.check_expression(&args[0])?;
                match arg_type {
                    Type::Array(_) | Type::String => Ok(Some(Type::Int)),
                    _ => Err(format!("len() expects array or string, got '{}'\n   = help: len() works on arrays and strings only.", arg_type.fmt_for_user())),
                }
            }

            "type" => {
                if args.len() != 1 {
                    return Err(format!("type() expects 1 argument, got {}", args.len()));
                }

                self.check_expression(&args[0])?;
                Ok(Some(Type::String))
            }

            "print" => {
                if args.is_empty() {
                    return Err("print() expects at least 1 argument".to_string());
                }

                for arg in args {
                    self.check_expression(arg)?;
                }

                Ok(Some(Type::Void))
            }

            "input" => {
                if args.len() > 1 {
                    return Err(format!(
                        "input() expects 0 or 1 argument, got {}",
                        args.len()
                    ));
                }

                if args.len() == 1 {
                    let prompt_type = self.check_expression(&args[0])?;
                    if prompt_type != Type::String {
                        return Err("input() prompt must be a string".to_string());
                    }
                }

                Ok(Some(Type::String))
            }

            "read_file" => {
                if args.len() != 1 {
                    return Err(format!(
                        "read_file() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let filename_type = self.check_expression(&args[0])?;
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

                let filename_type = self.check_expression(&args[0])?;
                if filename_type != Type::String {
                    return Err("write_file() filename must be a string".to_string());
                }

                self.check_expression(&args[1])?;

                Ok(Some(Type::Void))
            }

            "append_file" => {
                if args.len() != 2 {
                    return Err(format!(
                        "append_file() expects 2 arguments, got {}",
                        args.len()
                    ));
                }

                let filename_type = self.check_expression(&args[0])?;
                if filename_type != Type::String {
                    return Err("append_file() filename must be a string".to_string());
                }

                self.check_expression(&args[1])?;

                Ok(Some(Type::Void))
            }

            "file_exists" => {
                if args.len() != 1 {
                    return Err(format!(
                        "file_exists() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let filename_type = self.check_expression(&args[0])?;
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

                let path_type = self.check_expression(&args[0])?;
                if path_type != Type::String {
                    return Err("list_directory() path must be a string".to_string());
                }

                Ok(Some(Type::Array(Box::new(Type::String))))
            }

            "create_directory" | "remove_file" | "remove_directory" => {
                if args.len() != 1 {
                    return Err(format!("{}() expects 1 argument, got {}", name, args.len()));
                }

                let path_type = self.check_expression(&args[0])?;
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

                let path_type = self.check_expression(&args[0])?;
                if path_type != Type::String {
                    return Err("get_file_size() path must be a string".to_string());
                }

                Ok(Some(Type::Int))
            }

            "is_dir" => {
                if args.len() != 1 {
                    return Err(format!("is_dir() expects 1 argument, got {}", args.len()));
                }

                let path_type = self.check_expression(&args[0])?;
                if path_type != Type::String {
                    return Err("is_dir() path must be a string".to_string());
                }

                Ok(Some(Type::Bool))
            }

            "sys_time" | "sys_date" => {
                if !args.is_empty() {
                    return Err(format!(
                        "{}() expects 0 arguments, got {}",
                        name,
                        args.len()
                    ));
                }

                Ok(Some(Type::String))
            }

            "parse_int" => {
                if args.len() != 1 {
                    return Err(format!(
                        "parse_int() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let arg_type = self.check_expression(&args[0])?;
                if arg_type != Type::String {
                    return Err("parse_int() expects a string argument".to_string());
                }

                Ok(Some(Type::Int))
            }

            "char_code" => {
                if args.len() != 1 {
                    return Err(format!(
                        "char_code() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let arg_type = self.check_expression(&args[0])?;
                if arg_type != Type::String {
                    return Err("char_code() expects a string argument".to_string());
                }

                Ok(Some(Type::Int))
            }

            "sys_timestamp" => {
                if !args.is_empty() {
                    return Err(format!(
                        "sys_timestamp() expects 0 arguments, got {}",
                        args.len()
                    ));
                }

                Ok(Some(Type::Float))
            }

            "format" => {
                if args.is_empty() {
                    return Err(format!(
                        "format() expects at least 1 argument, got {}",
                        args.len()
                    ));
                }

                let template_type = self.check_expression(&args[0])?;
                if template_type != Type::String {
                    return Err("format() template must be a string".to_string());
                }

                Ok(Some(Type::String))
            }

            "enum_from_string" => {
                if args.len() != 2 {
                    return Err(format!(
                        "enum_from_string() expects 2 arguments, got {}",
                        args.len()
                    ));
                }

                let enum_name_type = self.check_expression(&args[0])?;
                let variant_name_type = self.check_expression(&args[1])?;

                if enum_name_type != Type::String {
                    return Err("enum_from_string() first argument must be a string".to_string());
                }

                if variant_name_type != Type::String {
                    return Err("enum_from_string() second argument must be a string".to_string());
                }

                if let Expression::StringLiteral(enum_name) = &args[0] {
                    if self.enums.contains_key(enum_name) {
                        return Ok(Some(Type::Enum(enum_name.clone())));
                    }
                }

                Ok(Some(Type::Enum("Unknown".to_string())))
            }

            _ => Ok(None),
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

        self.modules.insert(module_name.to_string(), module_info);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ASTNode, Expression};

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
