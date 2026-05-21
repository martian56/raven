use crate::ast::{ASTNode, EnumMember, Expression, ImplMember, Operator, Parameter, StructMember};
use std::collections::HashMap;
use std::fs;
use std::net::{TcpListener, TcpStream};

mod array_ops;
mod builtins;
mod methods;
mod value;
pub use value::Value;

use array_ops::flatten_array_index_chain;

#[derive(Clone)]
pub struct Function {
    params: Vec<Parameter>,
    body: ASTNode,
}

#[derive(Clone)]
pub struct Module {
    pub variables: HashMap<String, Value>,
    pub functions: HashMap<String, Function>,
    pub exports: Vec<String>,
}

pub struct Interpreter {
    variables: HashMap<String, Value>,
    functions: HashMap<String, Function>,
    structs: HashMap<String, Vec<String>>,
    struct_field_types: HashMap<String, HashMap<String, String>>,
    struct_methods: HashMap<String, HashMap<String, Function>>,
    enums: HashMap<String, Vec<String>>,
    modules: HashMap<String, Module>,
    return_value: Option<Value>,
    tcp_listeners: HashMap<u64, TcpListener>,
    tcp_streams: HashMap<u64, TcpStream>,
    next_tcp_id: u64,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter {
            variables: HashMap::new(),
            functions: HashMap::new(),
            structs: HashMap::new(),
            struct_field_types: HashMap::new(),
            struct_methods: HashMap::new(),
            enums: HashMap::new(),
            modules: HashMap::new(),
            return_value: None,
            tcp_listeners: HashMap::new(),
            tcp_streams: HashMap::new(),
            next_tcp_id: 1,
        }
    }

    fn alloc_tcp_id(&mut self) -> u64 {
        let id = self.next_tcp_id;
        self.next_tcp_id += 1;
        id
    }

    pub fn execute(&mut self, node: &ASTNode) -> Result<Value, String> {
        if self.return_value.is_some() {
            return Ok(self.return_value.clone().unwrap());
        }

        match node {
            ASTNode::VariableDecl(name, expr) => {
                let value = self.eval_expression(expr)?;
                self.variables.insert(name.clone(), value);
                Ok(Value::Void)
            }

            ASTNode::VariableDeclTyped(name, type_str, expr) => {
                let value = match expr.as_ref() {
                    Expression::Uninitialized => self.default_value_for_type_str(type_str)?,
                    _ => self.eval_expression(expr)?,
                };
                self.variables.insert(name.clone(), value);
                Ok(Value::Void)
            }

            ASTNode::Assignment(target, expr) => {
                let value = self.eval_expression(expr)?;

                match target.as_ref() {
                    Expression::Identifier(name) => {
                        if self.variables.contains_key(name) {
                            self.variables.insert(name.clone(), value);
                            Ok(Value::Void)
                        } else {
                            Err(format!("Variable '{}' not declared", name))
                        }
                    }
                    Expression::FieldAccess(object, field_name) => {
                        let _object_value = self.eval_expression(object)?;

                        match object.as_ref() {
                            Expression::Identifier(var_name) => {
                                if let Some(Value::Struct(_, ref mut fields)) =
                                    self.variables.get_mut(var_name)
                                {
                                    fields.insert(field_name.clone(), value);
                                    Ok(Value::Void)
                                } else {
                                    Err(format!("Variable '{}' is not a struct", var_name))
                                }
                            }
                            _ => Err("Cannot assign to complex field expression".to_string()),
                        }
                    }
                    Expression::ArrayIndex(_, _) => {
                        if let Some((root, indices)) = flatten_array_index_chain(target.as_ref()) {
                            self.assign_array_flat_target(root, &indices, value)?;
                            Ok(Value::Void)
                        } else {
                            Err("Cannot assign to this array expression".to_string())
                        }
                    }
                    _ => Ok(Value::Void),
                }
            }

            ASTNode::FunctionDecl(name, _return_type, params, body) => {
                self.functions.insert(
                    name.clone(),
                    Function {
                        params: params.clone(),
                        body: (**body).clone(),
                    },
                );
                Ok(Value::Void)
            }

            ASTNode::StructDecl(name, members) => {
                let mut field_names = Vec::new();
                let mut types = HashMap::new();
                for m in members {
                    match m {
                        StructMember::Field(f) => {
                            field_names.push(f.name.clone());
                            types.insert(f.name.clone(), f.field_type.clone());
                        }
                        StructMember::Comment(_) => {}
                    }
                }
                self.struct_field_types.insert(name.clone(), types);
                self.structs.insert(name.clone(), field_names);
                Ok(Value::Void)
            }

            ASTNode::ImplBlock(struct_name, methods) => {
                for method in methods {
                    match method {
                        ImplMember::Method(method_name, _return_type, params, body) => {
                            let func = Function {
                                params: params.clone(),
                                body: (**body).clone(),
                            };
                            self.struct_methods
                                .entry(struct_name.clone())
                                .or_default()
                                .insert(method_name.clone(), func);
                        }
                        ImplMember::Comment(_) => {}
                    }
                }
                Ok(Value::Void)
            }

            ASTNode::EnumDecl(name, members) => {
                let variants: Vec<String> = members
                    .iter()
                    .filter_map(|m| match m {
                        EnumMember::Variant(v) => Some(v.clone()),
                        EnumMember::Comment(_) => None,
                    })
                    .collect();
                self.enums.insert(name.clone(), variants);
                Ok(Value::Void)
            }

            ASTNode::Comment(_) => Ok(Value::Void),

            ASTNode::IfStatement(condition, then_block, else_if, else_block) => {
                let cond_value = self.eval_expression(condition)?;

                if let Value::Bool(true) = cond_value {
                    self.execute(then_block)
                } else if let Some(else_if_node) = else_if {
                    self.execute(else_if_node)
                } else if let Some(else_node) = else_block {
                    self.execute(else_node)
                } else {
                    Ok(Value::Void)
                }
            }

            ASTNode::WhileLoop(condition, body) => {
                loop {
                    let cond_value = self.eval_expression(condition)?;

                    if let Value::Bool(true) = cond_value {
                        self.execute(body)?;

                        if self.return_value.is_some() {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                Ok(Value::Void)
            }

            ASTNode::ForLoop(init, condition, increment, body) => {
                self.execute(init)?;

                loop {
                    let cond_value = self.eval_expression(condition)?;

                    if let Value::Bool(true) = cond_value {
                        self.execute(body)?;

                        if self.return_value.is_some() {
                            break;
                        }

                        self.execute(increment)?;
                    } else {
                        break;
                    }
                }
                Ok(Value::Void)
            }

            ASTNode::Block(statements) => {
                let mut last_value = Value::Void;
                for stmt in statements {
                    last_value = self.execute(stmt)?;

                    if self.return_value.is_some() {
                        break;
                    }
                }
                Ok(last_value)
            }

            ASTNode::Print(expr) => {
                let value = self.eval_expression(expr)?;
                println!("{}", value);
                Ok(Value::Void)
            }

            ASTNode::FunctionCall(name, args) => {
                if let Some(result) = self.call_builtin_function(name, args)? {
                    return Ok(result);
                }

                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_expression(arg)?);
                }

                self.call_function(name, evaluated_args)
            }

            ASTNode::MethodCall(object, method_name, args) => {
                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_expression(arg)?);
                }

                if let Expression::Identifier(var_name) = object.as_ref() {
                    if let Some(Value::Array(mut elements)) = self.variables.get(var_name).cloned()
                    {
                        match method_name.as_str() {
                            "push" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!(
                                        "push() expects 1 argument, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                elements.push(evaluated_args[0].clone());
                                self.variables
                                    .insert(var_name.clone(), Value::Array(elements));
                                Ok(Value::Void)
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
                            _ => self.eval_expression(&Expression::MethodCall(
                                object.clone(),
                                method_name.clone(),
                                args.clone(),
                            )),
                        }
                    } else {
                        Err(format!("Variable '{}' is not an array", var_name))
                    }
                } else {
                    self.eval_expression(&Expression::MethodCall(
                        object.clone(),
                        method_name.clone(),
                        args.clone(),
                    ))
                }
            }

            ASTNode::ExpressionStatement(expr) => {
                self.eval_expression(expr)?;
                Ok(Value::Void)
            }

            ASTNode::Import(module_name, alias) => {
                self.load_module(module_name)?;

                let var_name = alias.as_ref().unwrap_or(module_name);
                self.variables
                    .insert(var_name.clone(), Value::Module(module_name.clone()));

                Ok(Value::Void)
            }

            ASTNode::ImportSelective(module_name, items) => {
                self.load_module(module_name)?;

                if let Some(module) = self.modules.get(module_name) {
                    for item in items {
                        if let Some(value) = module.variables.get(item) {
                            self.variables.insert(item.clone(), value.clone());
                        } else if let Some(func) = module.functions.get(item) {
                            self.functions.insert(item.clone(), func.clone());
                        } else {
                            let available: Vec<String> = module
                                .variables
                                .keys()
                                .chain(module.functions.keys())
                                .cloned()
                                .collect();
                            return Err(format!(
                                "Item '{}' not found in module '{}'\n   = help: Available: {}",
                                item,
                                module_name,
                                available.join(", ")
                            ));
                        }
                    }
                } else {
                    return Err(format!("Module '{}' not found", module_name));
                }

                Ok(Value::Void)
            }

            ASTNode::Export(stmt) => self.execute(stmt),

            ASTNode::Return(expr) => {
                let value = self.eval_expression(expr)?;
                self.return_value = Some(value.clone());
                Ok(value)
            }
        }
    }

    fn default_value_for_type_str(&self, type_str: &str) -> Result<Value, String> {
        match type_str {
            "int" => Ok(Value::Int(0)),
            "float" => Ok(Value::Float(0.0)),
            "bool" => Ok(Value::Bool(false)),
            "string" => Ok(Value::String(String::new())),
            "TcpListener" | "TcpStream" => Err(
                "TcpListener and TcpStream cannot be default-initialized; use tcp_listen / tcp_accept"
                    .to_string(),
            ),
            s if s.ends_with("[]") => Ok(Value::Array(vec![])),
            struct_name => {
                let field_names = self.structs.get(struct_name).ok_or_else(|| {
                    format!(
                        "Unknown type '{}' for default-initialized variable",
                        struct_name
                    )
                })?;
                let types = self.struct_field_types.get(struct_name).ok_or_else(|| {
                    format!(
                        "Missing field metadata for struct '{}' (internal error)",
                        struct_name
                    )
                })?;
                let mut fields = HashMap::new();
                for fname in field_names {
                    let ftype = types.get(fname).ok_or_else(|| {
                        format!(
                            "Missing type for field '{}' in struct '{}'",
                            fname, struct_name
                        )
                    })?;
                    fields.insert(fname.clone(), self.default_value_for_type_str(ftype)?);
                }
                Ok(Value::Struct(struct_name.to_string(), fields))
            }
        }
    }

    fn eval_expression(&mut self, expr: &Expression) -> Result<Value, String> {
        match expr {
            Expression::Uninitialized => {
                Err("Evaluated uninitialized placeholder (internal error)".to_string())
            }

            Expression::Integer(i) => Ok(Value::Int(*i)),
            Expression::Float(f) => Ok(Value::Float(*f)),
            Expression::Boolean(b) => Ok(Value::Bool(*b)),
            Expression::StringLiteral(s) => Ok(Value::String(s.clone())),

            Expression::Identifier(name) => {
                if let Some(value) = self.variables.get(name) {
                    Ok(value.clone())
                } else {
                    Err(format!("Variable '{}' not declared", name))
                }
            }

            Expression::UnaryOp(op, expr) => {
                let expr_val = self.eval_expression(expr)?;

                match (op, &expr_val) {
                    (Operator::UnaryMinus, Value::Int(i)) => Ok(Value::Int(-i)),
                    (Operator::UnaryMinus, Value::Float(f)) => Ok(Value::Float(-f)),
                    (Operator::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    _ => Err(format!(
                        "Invalid unary operation: {:?} on {:?}",
                        op, expr_val
                    )),
                }
            }

            Expression::BinaryOp(left, op, right) => {
                let left_val = self.eval_expression(left)?;
                let right_val = self.eval_expression(right)?;

                match (left_val, op, right_val) {
                    (Value::Int(l), Operator::Add, Value::Int(r)) => Ok(Value::Int(l + r)),
                    (Value::Int(l), Operator::Subtract, Value::Int(r)) => Ok(Value::Int(l - r)),
                    (Value::Int(l), Operator::Multiply, Value::Int(r)) => Ok(Value::Int(l * r)),
                    (Value::Int(l), Operator::Divide, Value::Int(r)) => {
                        if r == 0 {
                            Err("Division by zero".to_string())
                        } else {
                            Ok(Value::Int(l / r))
                        }
                    }
                    (Value::Int(l), Operator::Modulo, Value::Int(r)) => {
                        if r == 0 {
                            Err("Modulo by zero".to_string())
                        } else {
                            Ok(Value::Int(l % r))
                        }
                    }

                    (Value::Float(l), Operator::Add, Value::Float(r)) => Ok(Value::Float(l + r)),
                    (Value::Float(l), Operator::Subtract, Value::Float(r)) => {
                        Ok(Value::Float(l - r))
                    }
                    (Value::Float(l), Operator::Multiply, Value::Float(r)) => {
                        Ok(Value::Float(l * r))
                    }
                    (Value::Float(l), Operator::Divide, Value::Float(r)) => {
                        if r == 0.0 {
                            Err("Division by zero".to_string())
                        } else {
                            Ok(Value::Float(l / r))
                        }
                    }
                    (Value::Float(l), Operator::Modulo, Value::Float(r)) => {
                        if r == 0.0 {
                            Err("Modulo by zero".to_string())
                        } else {
                            Ok(Value::Float(l % r))
                        }
                    }

                    (Value::Int(l), Operator::Add, Value::Float(r)) => {
                        Ok(Value::Float(l as f64 + r))
                    }
                    (Value::Float(l), Operator::Add, Value::Int(r)) => {
                        Ok(Value::Float(l + r as f64))
                    }
                    (Value::Int(l), Operator::Subtract, Value::Float(r)) => {
                        Ok(Value::Float(l as f64 - r))
                    }
                    (Value::Float(l), Operator::Subtract, Value::Int(r)) => {
                        Ok(Value::Float(l - r as f64))
                    }
                    (Value::Int(l), Operator::Multiply, Value::Float(r)) => {
                        Ok(Value::Float(l as f64 * r))
                    }
                    (Value::Float(l), Operator::Multiply, Value::Int(r)) => {
                        Ok(Value::Float(l * r as f64))
                    }
                    (Value::Int(l), Operator::Divide, Value::Float(r)) => {
                        if r == 0.0 {
                            Err("Division by zero".to_string())
                        } else {
                            Ok(Value::Float(l as f64 / r))
                        }
                    }
                    (Value::Float(l), Operator::Divide, Value::Int(r)) => {
                        if r == 0 {
                            Err("Division by zero".to_string())
                        } else {
                            Ok(Value::Float(l / r as f64))
                        }
                    }
                    (Value::Int(l), Operator::Modulo, Value::Float(r)) => {
                        if r == 0.0 {
                            Err("Modulo by zero".to_string())
                        } else {
                            Ok(Value::Float(l as f64 % r))
                        }
                    }
                    (Value::Float(l), Operator::Modulo, Value::Int(r)) => {
                        if r == 0 {
                            Err("Modulo by zero".to_string())
                        } else {
                            Ok(Value::Float(l % r as f64))
                        }
                    }

                    (Value::String(l), Operator::Add, Value::String(r)) => {
                        Ok(Value::String(format!("{}{}", l, r)))
                    }
                    (Value::String(l), Operator::Add, Value::Int(r)) => {
                        Ok(Value::String(format!("{}{}", l, r)))
                    }
                    (Value::Int(l), Operator::Add, Value::String(r)) => {
                        Ok(Value::String(format!("{}{}", l, r)))
                    }
                    (Value::String(l), Operator::Add, Value::Float(r)) => {
                        Ok(Value::String(format!("{}{}", l, r)))
                    }
                    (Value::Float(l), Operator::Add, Value::String(r)) => {
                        Ok(Value::String(format!("{}{}", l, r)))
                    }

                    (Value::Int(l), Operator::Equal, Value::Int(r)) => Ok(Value::Bool(l == r)),
                    (Value::Int(l), Operator::NotEqual, Value::Int(r)) => Ok(Value::Bool(l != r)),
                    (Value::Int(l), Operator::LessThan, Value::Int(r)) => Ok(Value::Bool(l < r)),
                    (Value::Int(l), Operator::GreaterThan, Value::Int(r)) => Ok(Value::Bool(l > r)),
                    (Value::Int(l), Operator::LessEqual, Value::Int(r)) => Ok(Value::Bool(l <= r)),
                    (Value::Int(l), Operator::GreaterEqual, Value::Int(r)) => {
                        Ok(Value::Bool(l >= r))
                    }

                    (Value::Float(l), Operator::Equal, Value::Float(r)) => Ok(Value::Bool(l == r)),
                    (Value::Float(l), Operator::NotEqual, Value::Float(r)) => {
                        Ok(Value::Bool(l != r))
                    }
                    (Value::Float(l), Operator::LessThan, Value::Float(r)) => {
                        Ok(Value::Bool(l < r))
                    }
                    (Value::Float(l), Operator::GreaterThan, Value::Float(r)) => {
                        Ok(Value::Bool(l > r))
                    }
                    (Value::Float(l), Operator::LessEqual, Value::Float(r)) => {
                        Ok(Value::Bool(l <= r))
                    }
                    (Value::Float(l), Operator::GreaterEqual, Value::Float(r)) => {
                        Ok(Value::Bool(l >= r))
                    }

                    (Value::Bool(l), Operator::And, Value::Bool(r)) => Ok(Value::Bool(l && r)),
                    (Value::Bool(l), Operator::Or, Value::Bool(r)) => Ok(Value::Bool(l || r)),
                    (Value::Bool(l), Operator::Equal, Value::Bool(r)) => Ok(Value::Bool(l == r)),
                    (Value::Bool(l), Operator::NotEqual, Value::Bool(r)) => Ok(Value::Bool(l != r)),

                    (Value::String(l), Operator::Equal, Value::String(r)) => {
                        Ok(Value::Bool(l == r))
                    }
                    (Value::String(l), Operator::NotEqual, Value::String(r)) => {
                        Ok(Value::Bool(l != r))
                    }

                    (Value::TcpListener(l), Operator::Equal, Value::TcpListener(r)) => {
                        Ok(Value::Bool(l == r))
                    }
                    (Value::TcpListener(l), Operator::NotEqual, Value::TcpListener(r)) => {
                        Ok(Value::Bool(l != r))
                    }
                    (Value::TcpStream(l), Operator::Equal, Value::TcpStream(r)) => {
                        Ok(Value::Bool(l == r))
                    }
                    (Value::TcpStream(l), Operator::NotEqual, Value::TcpStream(r)) => {
                        Ok(Value::Bool(l != r))
                    }

                    _ => Err(format!(
                        "Type error in binary operation: {:?} {:?}",
                        left, right
                    )),
                }
            }

            Expression::FunctionCall(name, args) => {
                if let Some(result) = self.call_builtin_function(name, args)? {
                    return Ok(result);
                }

                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_expression(arg)?);
                }

                self.call_function(name, evaluated_args)
            }

            Expression::ArrayLiteral(elements) => {
                let mut array_elements = Vec::new();
                for element in elements {
                    array_elements.push(self.eval_expression(element)?);
                }
                Ok(Value::Array(array_elements))
            }

            Expression::ArrayIndex(array_expr, index_expr) => {
                let array = self.eval_expression(array_expr)?;
                let index = self.eval_expression(index_expr)?;

                let index_int = match index {
                    Value::Int(i) => i,
                    _ => return Err("Array index must be integer".to_string()),
                };

                match array {
                    Value::Array(elements) => {
                        if index_int < 0 || index_int as usize >= elements.len() {
                            return Err(format!(
                                "Array index {} out of bounds (array length: {})",
                                index_int,
                                elements.len()
                            ));
                        }
                        Ok(elements[index_int as usize].clone())
                    }
                    Value::String(s) => {
                        if index_int < 0 || index_int as usize >= s.len() {
                            return Err(format!(
                                "String index {} out of bounds (string length: {})",
                                index_int,
                                s.len()
                            ));
                        }
                        let ch = s
                            .chars()
                            .nth(index_int as usize)
                            .ok_or_else(|| "Invalid character index".to_string())?;
                        Ok(Value::String(ch.to_string()))
                    }
                    _ => Err("Cannot index non-array or non-string value".to_string()),
                }
            }

            Expression::MethodCall(object_expr, method_name, args) => {
                self.eval_method_call(object_expr, method_name, args)
            }

            Expression::StructInstantiation(struct_name, fields) => {
                if let Some(field_names) = self.structs.get(struct_name) {
                    let field_names_clone = field_names.clone();
                    let mut field_values = HashMap::new();

                    for (field_name, field_expr) in fields {
                        let field_value = self.eval_expression(field_expr)?;
                        field_values.insert(field_name.clone(), field_value);
                    }

                    for field_name in &field_names_clone {
                        if !field_values.contains_key(field_name) {
                            return Err(format!(
                                "Missing required field '{}' in struct '{}'",
                                field_name, struct_name
                            ));
                        }
                    }

                    Ok(Value::Struct(struct_name.clone(), field_values))
                } else {
                    Err(format!("Struct '{}' not declared", struct_name))
                }
            }

            Expression::FieldAccess(object_expr, field_name) => {
                let object = self.eval_expression(object_expr)?;

                if let Value::Struct(_, fields) = object {
                    if let Some(field_value) = fields.get(field_name) {
                        Ok(field_value.clone())
                    } else {
                        Err(format!("Field '{}' not found in struct", field_name))
                    }
                } else {
                    Err(format!(
                        "Cannot access field on non-struct value of type {:?}",
                        object
                    ))
                }
            }

            Expression::EnumVariant(enum_name, variant_name) => {
                if let Some(variants) = self.enums.get(enum_name) {
                    if variants.contains(variant_name) {
                        Ok(Value::Enum(enum_name.clone(), variant_name.clone()))
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

    fn call_struct_method(
        &mut self,
        struct_value: Value,
        method_name: &str,
        args: Vec<Value>,
        update_var: Option<String>,
    ) -> Result<Value, String> {
        let (struct_name, _) = match &struct_value {
            Value::Struct(n, f) => (n.clone(), f.clone()),
            _ => {
                return Err("Expected struct value for method call".to_string());
            }
        };

        if let Some(methods) = self.struct_methods.get(&struct_name) {
            if let Some(func) = methods.get(method_name).cloned() {
                let mut full_args = vec![struct_value];
                full_args.extend(args);

                if func.params.len() != full_args.len() {
                    return Err(format!(
                        "Method '{}' expects {} arguments, got {}",
                        method_name,
                        func.params.len(),
                        full_args.len()
                    ));
                }

                let saved_vars = self.variables.clone();
                for (i, param) in func.params.iter().enumerate() {
                    self.variables
                        .insert(param.name.clone(), full_args[i].clone());
                }

                self.return_value = None;
                self.execute(&func.body)?;
                let result = self.return_value.clone().unwrap_or(Value::Void);
                let modified_self = self.variables.get("self").cloned();
                self.return_value = None;
                self.variables = saved_vars;

                if let (Some(var_name), Some(modified)) = (update_var, modified_self) {
                    self.variables.insert(var_name, modified);
                }

                return Ok(result);
            }
        }

        Err(format!(
            "Method '{}' not found on struct '{}'",
            method_name, struct_name
        ))
    }

    pub fn call_function(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String> {
        if let Some(func) = self.functions.get(name).cloned() {
            if func.params.len() != args.len() {
                return Err(format!(
                    "Function '{}' expects {} arguments, got {}",
                    name,
                    func.params.len(),
                    args.len()
                ));
            }

            let saved_vars = self.variables.clone();

            for (i, param) in func.params.iter().enumerate() {
                self.variables.insert(param.name.clone(), args[i].clone());
            }

            self.return_value = None;
            self.execute(&func.body)?;

            let result = self.return_value.clone().unwrap_or(Value::Void);
            self.return_value = None;

            self.variables = saved_vars;

            Ok(result)
        } else {
            Err(format!("Function '{}' not found", name))
        }
    }

    fn load_module(&mut self, module_name: &str) -> Result<(), String> {
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

        let mut module_interpreter = Interpreter::new();

        module_interpreter.execute(&ast)?;

        let nested_modules_snapshot = module_interpreter.modules.clone();

        let module = Module {
            variables: module_interpreter.variables,
            functions: module_interpreter.functions,
            exports: Vec::new(),
        };

        for (name, func) in &module.functions {
            self.functions.insert(name.clone(), func.clone());
        }

        for (name, fields) in &module_interpreter.structs {
            self.structs.insert(name.clone(), fields.clone());
        }

        for (name, types) in &module_interpreter.struct_field_types {
            self.struct_field_types.insert(name.clone(), types.clone());
        }

        for (struct_name, methods) in &module_interpreter.struct_methods {
            for (method_name, func) in methods {
                self.struct_methods
                    .entry(struct_name.clone())
                    .or_default()
                    .insert(method_name.clone(), func.clone());
            }
        }

        self.modules.insert(module_name.to_string(), module);

        for (nested_name, nested_mod) in nested_modules_snapshot {
            self.modules.entry(nested_name).or_insert(nested_mod);
        }

        Ok(())
    }

    fn call_function_with_module(
        &mut self,
        func: &Function,
        args: Vec<Value>,
        module: &Module,
    ) -> Result<Value, String> {
        let mut function_variables = self.variables.clone();

        for (name, value) in &module.variables {
            function_variables.insert(name.clone(), value.clone());
        }

        if args.len() != func.params.len() {
            return Err(format!(
                "Function expects {} arguments, got {}",
                func.params.len(),
                args.len()
            ));
        }

        for (i, param) in func.params.iter().enumerate() {
            function_variables.insert(param.name.clone(), args[i].clone());
        }

        let old_variables = std::mem::replace(&mut self.variables, function_variables);
        let old_return_value = self.return_value.take();

        self.return_value = None;
        self.execute(&func.body)?;
        let result = self.return_value.clone().unwrap_or(Value::Void);
        self.return_value = None;

        self.variables = old_variables;
        self.return_value = old_return_value;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Expression;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::type_checker::TypeChecker;

    #[test]
    fn test_struct_field_array_push_updates_struct() {
        let src = r#"
struct S { items: string[] }
let s: S = S { items: [] };
s.items.push("a");
"#;
        let lexer = Lexer::new(src.to_string());
        let mut parser = Parser::new(lexer, src.to_string());
        let ast = parser.parse().expect("parse");
        let mut checker = TypeChecker::new();
        checker.check(&ast).expect("typecheck");
        let mut interp = Interpreter::new();
        interp.execute(&ast).expect("run");

        let Some(Value::Struct(_, fields)) = interp.variables.get("s") else {
            panic!("expected s");
        };
        let Some(Value::Array(items)) = fields.get("items") else {
            panic!("expected items array");
        };
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Value::String(ref t) if t == "a"));
    }

    #[test]
    fn test_nested_array_assignment_and_read() {
        let src = r#"
let m: int[][] = [[1, 2], [3, 4]];
m[0][1] = 9;
let m2: int[][][] = [[[1]], [[2]]];
m2[0][0][0] = 7;
"#;
        let lexer = Lexer::new(src.to_string());
        let mut parser = Parser::new(lexer, src.to_string());
        let ast = parser.parse().expect("parse");
        let mut checker = TypeChecker::new();
        checker.check(&ast).expect("typecheck");
        let mut interp = Interpreter::new();
        interp.execute(&ast).expect("run");

        if let Some(Value::Array(rows)) = interp.variables.get("m") {
            assert_eq!(rows.len(), 2);
            if let Value::Array(r0) = &rows[0] {
                assert!(matches!(r0[1], Value::Int(9)));
            } else {
                panic!("expected row 0 to be array");
            }
        } else {
            panic!("expected m");
        }

        if let Some(Value::Array(planes)) = interp.variables.get("m2") {
            if let Value::Array(rows) = &planes[0] {
                if let Value::Array(cells) = &rows[0] {
                    assert!(matches!(cells[0], Value::Int(7)));
                } else {
                    panic!("expected depth 3");
                }
            } else {
                panic!("expected depth 2");
            }
        } else {
            panic!("expected m2");
        }
    }

    #[test]
    fn test_variable_assignment() {
        let mut interp = Interpreter::new();
        let node = ASTNode::VariableDeclTyped(
            "x".to_string(),
            "int".to_string(),
            Box::new(Expression::Integer(42)),
        );

        assert!(interp.execute(&node).is_ok());
        assert_eq!(interp.variables.get("x").unwrap().to_string(), "42");
    }

    #[test]
    fn test_arithmetic() {
        let mut interp = Interpreter::new();
        let expr = Expression::BinaryOp(
            Box::new(Expression::Integer(10)),
            Operator::Add,
            Box::new(Expression::Integer(5)),
        );

        let result = interp.eval_expression(&expr).unwrap();
        if let Value::Int(v) = result {
            assert_eq!(v, 15);
        } else {
            panic!("Expected integer result");
        }
    }

    #[test]
    fn test_print() {
        let mut interp = Interpreter::new();
        let node = ASTNode::Print(Box::new(Expression::StringLiteral(
            "Hello, Raven!".to_string(),
        )));

        assert!(interp.execute(&node).is_ok());
    }
}
