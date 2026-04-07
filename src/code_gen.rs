use crate::ast::{ASTNode, EnumMember, Expression, ImplMember, Operator, Parameter, StructMember};
use chrono::Local;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

fn flatten_array_index_chain(expr: &Expression) -> Option<(&Expression, Vec<&Expression>)> {
    let mut indices = Vec::new();
    let mut e = expr;
    loop {
        match e {
            Expression::ArrayIndex(array_expr, index_expr) => {
                indices.push(index_expr.as_ref());
                e = array_expr.as_ref();
            }
            Expression::Identifier(_) | Expression::FieldAccess(..) => {
                indices.reverse();
                return Some((e, indices));
            }
            _ => return None,
        }
    }
}

fn assign_array_element_by_path(
    value: &mut Value,
    indices: &[usize],
    final_value: Value,
) -> Result<(), String> {
    if indices.is_empty() {
        return Err("Array assignment requires at least one index".to_string());
    }

    let mut current = value;
    for (depth, &idx) in indices.iter().enumerate() {
        if depth == indices.len() - 1 {
            match current {
                Value::Array(elements) => {
                    if idx < elements.len() {
                        elements[idx] = final_value;
                        return Ok(());
                    }
                    return Err(format!(
                        "Array index {} out of bounds (array length: {})",
                        idx,
                        elements.len()
                    ));
                }
                _ => return Err("Cannot assign through non-array value".to_string()),
            }
        } else {
            match current {
                Value::Array(elements) => {
                    if idx < elements.len() {
                        current = &mut elements[idx];
                    } else {
                        return Err(format!(
                            "Array index {} out of bounds (array length: {})",
                            idx,
                            elements.len()
                        ));
                    }
                }
                _ => return Err("Cannot index non-array value".to_string()),
            }
        }
    }
    unreachable!()
}

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<Value>),
    Struct(String, HashMap<String, Value>),
    Enum(String, String),
    Module(String),
    Void,
    TcpListener(u64),
    TcpStream(u64),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::Bool(b) => write!(f, "{}", b),
            Value::String(s) => write!(f, "{}", s),
            Value::Array(elements) => {
                write!(f, "[")?;
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", element)?;
                }
                write!(f, "]")
            }
            Value::Struct(name, fields) => {
                write!(f, "{} {{", name)?;
                for (i, (field_name, field_value)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", field_name, field_value)?;
                }
                write!(f, "}}")
            }
            Value::Enum(enum_name, variant_name) => {
                write!(f, "{}::{}", enum_name, variant_name)
            }
            Value::Module(name) => write!(f, "<module: {}>", name),
            Value::Void => write!(f, "void"),
            Value::TcpListener(id) => write!(f, "<TcpListener {}>", id),
            Value::TcpStream(id) => write!(f, "<TcpStream {}>", id),
        }
    }
}

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

    fn assign_array_flat_target(
        &mut self,
        root: &Expression,
        indices: &[&Expression],
        value: Value,
    ) -> Result<(), String> {
        let index_vals: Vec<usize> = indices
            .iter()
            .map(|e| match self.eval_expression(e)? {
                Value::Int(i) if i >= 0 => Ok(i as usize),
                _ => Err("Array index must be a non-negative integer".to_string()),
            })
            .collect::<Result<Vec<_>, String>>()?;

        match root {
            Expression::Identifier(name) => {
                if let Some(v) = self.variables.get_mut(name) {
                    assign_array_element_by_path(v, &index_vals, value)
                } else {
                    Err(format!("Variable '{}' not declared", name))
                }
            }
            Expression::FieldAccess(obj, field) => match obj.as_ref() {
                Expression::Identifier(obj_name) => {
                    if let Some(Value::Struct(_, ref mut fields)) = self.variables.get_mut(obj_name)
                    {
                        if let Some(v) = fields.get_mut(field) {
                            assign_array_element_by_path(v, &index_vals, value)
                        } else {
                            Err(format!("Field '{}' not found on struct", field))
                        }
                    } else {
                        Err(format!("Variable '{}' is not a struct", obj_name))
                    }
                }
                _ => Err("Cannot assign to complex field array expression".to_string()),
            },
            _ => Err("Invalid array assignment target".to_string()),
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
                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_expression(arg)?);
                }

                // map.keys.push(x) / map.keys.pop() — mutate array stored in a struct field (not
                // only a standalone variable). Without this, push returns a new array but the Map
                // struct is never updated (collections.rv map_set/map_remove rely on this).
                if let Expression::FieldAccess(inner, field_name) = object_expr.as_ref() {
                    if let Expression::Identifier(var_name) = inner.as_ref() {
                        if method_name == "push" || method_name == "pop" {
                            if let Some(Value::Struct(_, ref mut fields)) =
                                self.variables.get_mut(var_name)
                            {
                                if let Some(Value::Array(mut elements)) =
                                    fields.get(field_name).cloned()
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
                                            fields.insert(
                                                field_name.clone(),
                                                Value::Array(elements.clone()),
                                            );
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
                                                return Err(
                                                    "Cannot pop from empty array".to_string(),
                                                );
                                            }
                                            let popped = elements.pop().unwrap();
                                            fields.insert(
                                                field_name.clone(),
                                                Value::Array(elements),
                                            );
                                            return Ok(popped);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }

                if let Expression::Identifier(var_name) = object_expr.as_ref() {
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
                                    _ => {
                                        return Err(
                                            "slice() start index must be integer".to_string()
                                        )
                                    }
                                };
                                let end = match &evaluated_args[1] {
                                    Value::Int(i) => *i,
                                    _ => {
                                        return Err("slice() end index must be integer".to_string())
                                    }
                                };

                                if start < 0
                                    || end < 0
                                    || start > end
                                    || start as usize >= elements.len()
                                {
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

                                let strings: Vec<String> =
                                    elements.iter().map(|v| v.to_string()).collect();

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
                                self.call_function_with_module(
                                    &func_clone,
                                    evaluated_args,
                                    &module_clone,
                                )
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
                    } else if let Some(Value::String(s)) = self.variables.get(var_name) {
                        match method_name.as_str() {
                            "slice" => {
                                if evaluated_args.len() != 2 {
                                    return Err(format!(
                                        "slice() expects 2 arguments, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                let start = match &evaluated_args[0] {
                                    Value::Int(i) => *i,
                                    _ => {
                                        return Err(
                                            "slice() start index must be integer".to_string()
                                        )
                                    }
                                };
                                let end = match &evaluated_args[1] {
                                    Value::Int(i) => *i,
                                    _ => {
                                        return Err("slice() end index must be integer".to_string())
                                    }
                                };

                                if start < 0 || end < 0 || start > end || start as usize >= s.len()
                                {
                                    return Err("Invalid slice indices".to_string());
                                }

                                let start_idx = start as usize;
                                let end_idx = (end as usize).min(s.len());

                                Ok(Value::String(s[start_idx..end_idx].to_string()))
                            }
                            "split" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!(
                                        "split() expects 1 argument, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                let delimiter = match &evaluated_args[0] {
                                    Value::String(d) => d,
                                    _ => return Err("split() delimiter must be string".to_string()),
                                };

                                let parts: Vec<Value> = s
                                    .split(delimiter)
                                    .map(|part| Value::String(part.to_string()))
                                    .collect();

                                Ok(Value::Array(parts))
                            }
                            "replace" => {
                                if evaluated_args.len() != 2 {
                                    return Err(format!(
                                        "replace() expects 2 arguments, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                let from = match &evaluated_args[0] {
                                    Value::String(f) => f,
                                    _ => return Err("replace() 'from' must be string".to_string()),
                                };
                                let to = match &evaluated_args[1] {
                                    Value::String(t) => t,
                                    _ => return Err("replace() 'to' must be string".to_string()),
                                };

                                Ok(Value::String(s.replace(from, to)))
                            }
                            "index_of" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!(
                                        "index_of() expects 1 argument, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                let sub = match &evaluated_args[0] {
                                    Value::String(x) => x.as_str(),
                                    _ => {
                                        return Err("index_of() argument must be string".to_string())
                                    }
                                };
                                let i = s.find(sub).map(|i| i as i64).unwrap_or(-1);
                                Ok(Value::Int(i))
                            }
                            "last_index_of" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!(
                                        "last_index_of() expects 1 argument, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                let sub = match &evaluated_args[0] {
                                    Value::String(x) => x.as_str(),
                                    _ => {
                                        return Err(
                                            "last_index_of() argument must be string".to_string()
                                        )
                                    }
                                };
                                let i = s.rfind(sub).map(|i| i as i64).unwrap_or(-1);
                                Ok(Value::Int(i))
                            }
                            _ => Err(format!("Unknown method '{}' for string", method_name)),
                        }
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
                        match method_name.as_str() {
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
                                    _ => {
                                        return Err(
                                            "slice() start index must be integer".to_string()
                                        )
                                    }
                                };
                                let end = match &evaluated_args[1] {
                                    Value::Int(i) => *i,
                                    _ => {
                                        return Err("slice() end index must be integer".to_string())
                                    }
                                };

                                if start < 0
                                    || end < 0
                                    || start > end
                                    || start as usize >= elements.len()
                                {
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

                                let strings: Vec<String> =
                                    elements.iter().map(|v| v.to_string()).collect();

                                Ok(Value::String(strings.join(delimiter)))
                            }
                            _ => Err(format!("Unknown method '{}' for array", method_name)),
                        }
                    } else if let Value::String(s) = object {
                        match method_name.as_str() {
                            "slice" => {
                                if evaluated_args.len() != 2 {
                                    return Err(format!(
                                        "slice() expects 2 arguments, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                let start = match &evaluated_args[0] {
                                    Value::Int(i) => *i,
                                    _ => {
                                        return Err(
                                            "slice() start index must be integer".to_string()
                                        )
                                    }
                                };
                                let end = match &evaluated_args[1] {
                                    Value::Int(i) => *i,
                                    _ => {
                                        return Err("slice() end index must be integer".to_string())
                                    }
                                };

                                if start < 0 || end < 0 || start > end || start as usize >= s.len()
                                {
                                    return Err("Invalid slice indices".to_string());
                                }

                                let start_idx = start as usize;
                                let end_idx = (end as usize).min(s.len());

                                Ok(Value::String(s[start_idx..end_idx].to_string()))
                            }
                            "split" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!(
                                        "split() expects 1 argument, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                let delimiter = match &evaluated_args[0] {
                                    Value::String(d) => d,
                                    _ => return Err("split() delimiter must be string".to_string()),
                                };

                                let parts: Vec<Value> = s
                                    .split(delimiter)
                                    .map(|part| Value::String(part.to_string()))
                                    .collect();

                                Ok(Value::Array(parts))
                            }
                            "replace" => {
                                if evaluated_args.len() != 2 {
                                    return Err(format!(
                                        "replace() expects 2 arguments, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                let from = match &evaluated_args[0] {
                                    Value::String(f) => f,
                                    _ => return Err("replace() 'from' must be string".to_string()),
                                };
                                let to = match &evaluated_args[1] {
                                    Value::String(t) => t,
                                    _ => return Err("replace() 'to' must be string".to_string()),
                                };

                                Ok(Value::String(s.replace(from, to)))
                            }
                            "index_of" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!(
                                        "index_of() expects 1 argument, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                let sub = match &evaluated_args[0] {
                                    Value::String(x) => x.as_str(),
                                    _ => {
                                        return Err("index_of() argument must be string".to_string())
                                    }
                                };
                                let i = s.find(sub).map(|i| i as i64).unwrap_or(-1);
                                Ok(Value::Int(i))
                            }
                            "last_index_of" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!(
                                        "last_index_of() expects 1 argument, got {}",
                                        evaluated_args.len()
                                    ));
                                }
                                let sub = match &evaluated_args[0] {
                                    Value::String(x) => x.as_str(),
                                    _ => {
                                        return Err(
                                            "last_index_of() argument must be string".to_string()
                                        )
                                    }
                                };
                                let i = s.rfind(sub).map(|i| i as i64).unwrap_or(-1);
                                Ok(Value::Int(i))
                            }
                            _ => Err(format!("Unknown method '{}' for string", method_name)),
                        }
                    } else if let Value::Struct(..) = &object {
                        self.call_struct_method(object, method_name, evaluated_args, None)
                    } else {
                        Err(format!("Cannot call method on value of type {:?}", object))
                    }
                }
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

    fn value_from_ureq_response(resp: ureq::Response) -> Result<Value, String> {
        let status = resp.status() as i64;
        let status_text = resp.status_text().to_string();
        let mut header_strings: Vec<Value> = Vec::new();
        for name in resp.headers_names() {
            if let Some(v) = resp.header(&name) {
                header_strings.push(Value::String(format!("{}: {}", name, v)));
            }
        }
        let body_str = resp.into_string().unwrap_or_default();
        let mut fields = HashMap::new();
        fields.insert("status_code".to_string(), Value::Int(status));
        fields.insert("status_text".to_string(), Value::String(status_text));
        fields.insert("headers".to_string(), Value::Array(header_strings));
        fields.insert("body".to_string(), Value::String(body_str));
        Ok(Value::Struct("HttpResponse".to_string(), fields))
    }

    fn call_builtin_function(
        &mut self,
        name: &str,
        args: &[Expression],
    ) -> Result<Option<Value>, String> {
        match name {
            "len" => {
                if args.len() != 1 {
                    return Err(format!("len() expects 1 argument, got {}", args.len()));
                }

                let value = self.eval_expression(&args[0])?;
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

                let value = self.eval_expression(&args[0])?;
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

            "print" => {
                if args.is_empty() {
                    return Err("print() expects at least 1 argument".to_string());
                }

                if args.len() == 1 {
                    let value = self.eval_expression(&args[0])?;
                    println!("{}", value);
                } else {
                    let format_string = self.eval_expression(&args[0])?;
                    if let Value::String(format_str) = format_string {
                        let mut formatted = format_str.clone();

                        for (i, arg) in args.iter().enumerate().skip(1) {
                            let arg_value = self.eval_expression(arg)?;
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

            "panic" => {
                if args.is_empty() {
                    return Err("panic() expects at least 1 argument".to_string());
                }
                let mut parts = Vec::new();
                for arg in args {
                    let v = self.eval_expression(arg)?;
                    parts.push(v.to_string());
                }
                Err(parts.join(""))
            }

            "input" => {
                use std::io::{self, Write};

                if args.len() > 1 {
                    return Err(format!(
                        "input() expects 0 or 1 argument, got {}",
                        args.len()
                    ));
                }

                if args.len() == 1 {
                    let prompt = self.eval_expression(&args[0])?;
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

            "read_file" => {
                if args.len() != 1 {
                    return Err(format!(
                        "read_file() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let filename = self.eval_expression(&args[0])?;
                if let Value::String(filename_str) = filename {
                    match fs::read_to_string(&filename_str) {
                        Ok(content) => Ok(Some(Value::String(content))),
                        Err(e) => Err(format!("Error reading file '{}': {}", filename_str, e)),
                    }
                } else {
                    Err("read_file() filename must be a string".to_string())
                }
            }

            "write_file" => {
                if args.len() != 2 {
                    return Err(format!(
                        "write_file() expects 2 arguments, got {}",
                        args.len()
                    ));
                }

                let filename = self.eval_expression(&args[0])?;
                let content = self.eval_expression(&args[1])?;

                if let Value::String(filename_str) = filename {
                    let content_str = match content {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };

                    let processed_content = content_str.replace("\\n", "\n");

                    match fs::write(&filename_str, processed_content) {
                        Ok(_) => Ok(Some(Value::Void)),
                        Err(e) => Err(format!("Error writing file '{}': {}", filename_str, e)),
                    }
                } else {
                    Err("write_file() filename must be a string".to_string())
                }
            }

            "append_file" => {
                if args.len() != 2 {
                    return Err(format!(
                        "append_file() expects 2 arguments, got {}",
                        args.len()
                    ));
                }

                let filename = self.eval_expression(&args[0])?;
                let content = self.eval_expression(&args[1])?;

                if let Value::String(filename_str) = filename {
                    let content_str = match content {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };

                    let processed_content = content_str.replace("\\n", "\n");

                    match fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&filename_str)
                    {
                        Ok(mut file) => {
                            use std::io::Write;
                            match file.write_all(processed_content.as_bytes()) {
                                Ok(_) => Ok(Some(Value::Void)),
                                Err(e) => Err(format!(
                                    "Error appending to file '{}': {}",
                                    filename_str, e
                                )),
                            }
                        }
                        Err(e) => Err(format!("Error opening file '{}': {}", filename_str, e)),
                    }
                } else {
                    Err("append_file() filename must be a string".to_string())
                }
            }

            "file_exists" => {
                if args.len() != 1 {
                    return Err(format!(
                        "file_exists() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let filename = self.eval_expression(&args[0])?;
                if let Value::String(filename_str) = filename {
                    let exists = Path::new(&filename_str).exists();
                    Ok(Some(Value::Bool(exists)))
                } else {
                    Err("file_exists() filename must be a string".to_string())
                }
            }

            "list_directory" => {
                if args.len() != 1 {
                    return Err(format!(
                        "list_directory() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    match fs::read_dir(&path_str) {
                        Ok(entries) => {
                            let names: Vec<Value> = entries
                                .filter_map(|e| e.ok())
                                .filter_map(|e| e.file_name().into_string().ok())
                                .map(Value::String)
                                .collect();
                            Ok(Some(Value::Array(names)))
                        }
                        Err(_) => Ok(Some(Value::Array(vec![]))),
                    }
                } else {
                    Err("list_directory() path must be a string".to_string())
                }
            }

            "create_directory" => {
                if args.len() != 1 {
                    return Err(format!(
                        "create_directory() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    match fs::create_dir_all(&path_str) {
                        Ok(_) => Ok(Some(Value::Bool(true))),
                        Err(_) => Ok(Some(Value::Bool(false))),
                    }
                } else {
                    Err("create_directory() path must be a string".to_string())
                }
            }

            "remove_file" => {
                if args.len() != 1 {
                    return Err(format!(
                        "remove_file() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    match fs::remove_file(&path_str) {
                        Ok(_) => Ok(Some(Value::Bool(true))),
                        Err(_) => Ok(Some(Value::Bool(false))),
                    }
                } else {
                    Err("remove_file() path must be a string".to_string())
                }
            }

            "remove_directory" => {
                if args.len() != 1 {
                    return Err(format!(
                        "remove_directory() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    match fs::remove_dir_all(&path_str) {
                        Ok(_) => Ok(Some(Value::Bool(true))),
                        Err(_) => Ok(Some(Value::Bool(false))),
                    }
                } else {
                    Err("remove_directory() path must be a string".to_string())
                }
            }

            "get_file_size" => {
                if args.len() != 1 {
                    return Err(format!(
                        "get_file_size() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    match fs::metadata(&path_str) {
                        Ok(meta) => Ok(Some(Value::Int(meta.len() as i64))),
                        Err(_) => Ok(Some(Value::Int(0))),
                    }
                } else {
                    Err("get_file_size() path must be a string".to_string())
                }
            }

            "is_dir" => {
                if args.len() != 1 {
                    return Err(format!("is_dir() expects 1 argument, got {}", args.len()));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    let is_dir = Path::new(&path_str).is_dir();
                    Ok(Some(Value::Bool(is_dir)))
                } else {
                    Err("is_dir() path must be a string".to_string())
                }
            }

            "sys_time" => {
                if !args.is_empty() {
                    return Err(format!(
                        "sys_time() expects 0 arguments, got {}",
                        args.len()
                    ));
                }

                let now = Local::now();
                Ok(Some(Value::String(now.format("%H:%M:%S").to_string())))
            }

            "sys_date" => {
                if !args.is_empty() {
                    return Err(format!(
                        "sys_date() expects 0 arguments, got {}",
                        args.len()
                    ));
                }

                let now = Local::now();
                Ok(Some(Value::String(now.format("%Y-%m-%d").to_string())))
            }

            "parse_int" => {
                if args.len() != 1 {
                    return Err(format!(
                        "parse_int() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let val = self.eval_expression(&args[0])?;
                if let Value::String(s) = val {
                    match s.parse::<i64>() {
                        Ok(n) => Ok(Some(Value::Int(n))),
                        Err(_) => Ok(Some(Value::Int(0))),
                    }
                } else {
                    Err("parse_int() expects a string argument".to_string())
                }
            }

            "char_code" => {
                if args.len() != 1 {
                    return Err(format!(
                        "char_code() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let val = self.eval_expression(&args[0])?;
                if let Value::String(s) = val {
                    let code = s.chars().next().map(|c| c as i64).unwrap_or(0);
                    Ok(Some(Value::Int(code)))
                } else {
                    Err("char_code() expects a string argument".to_string())
                }
            }

            "sys_timestamp" => {
                if !args.is_empty() {
                    return Err(format!(
                        "sys_timestamp() expects 0 arguments, got {}",
                        args.len()
                    ));
                }

                let now = Local::now();
                Ok(Some(Value::Float(
                    now.timestamp() as f64 + now.timestamp_subsec_millis() as f64 / 1000.0,
                )))
            }

            "format" => {
                if args.is_empty() {
                    return Err(format!(
                        "format() expects at least 1 argument, got {}",
                        args.len()
                    ));
                }

                let template = self.eval_expression(&args[0])?;
                if let Value::String(template_str) = template {
                    let mut result = template_str.clone();
                    let mut arg_index = 1;

                    while let Some(pos) = result.find("{}") {
                        if arg_index >= args.len() {
                            return Err(
                                "format() not enough arguments for placeholders".to_string()
                            );
                        }

                        let replacement_value = self.eval_expression(&args[arg_index])?;
                        let replacement = replacement_value.to_string();
                        result.replace_range(pos..pos + 2, &replacement);
                        arg_index += 1;
                    }

                    Ok(Some(Value::String(result)))
                } else {
                    Err("format() template must be a string".to_string())
                }
            }

            "http_fetch" => {
                if args.len() != 4 {
                    return Err(format!(
                        "http_fetch() expects 4 arguments, got {}",
                        args.len()
                    ));
                }

                let method_val = self.eval_expression(&args[0])?;
                let url_val = self.eval_expression(&args[1])?;
                let headers_val = self.eval_expression(&args[2])?;
                let body_val = self.eval_expression(&args[3])?;

                let method = match method_val {
                    Value::String(s) => s,
                    _ => return Err("http_fetch() method must be a string".to_string()),
                };
                let url = match url_val {
                    Value::String(s) => s,
                    _ => return Err("http_fetch() url must be a string".to_string()),
                };
                let body = match body_val {
                    Value::String(s) => s,
                    other => other.to_string(),
                };

                let headers_vec: Vec<String> = match headers_val {
                    Value::Array(elements) => {
                        let mut v = Vec::new();
                        for e in elements {
                            match e {
                                Value::String(s) => v.push(s),
                                _ => {
                                    return Err("http_fetch() headers must be an array of strings"
                                        .to_string());
                                }
                            }
                        }
                        v
                    }
                    _ => return Err("http_fetch() headers must be string[]".to_string()),
                };

                let agent = ureq::Agent::new();
                let mut req = agent.request(method.trim(), &url).set(
                    "User-Agent",
                    "Raven/1.4 (+https://github.com/martian56/raven)",
                );
                for h in &headers_vec {
                    if let Some(colon) = h.find(':') {
                        let hn = h[..colon].trim();
                        let hv = h[colon + 1..].trim();
                        if !hn.is_empty() {
                            req = req.set(hn, hv);
                        }
                    }
                }

                let resp_result = if body.is_empty() {
                    req.call()
                } else {
                    req.send_string(&body)
                };

                match resp_result {
                    Ok(resp) => Ok(Some(Self::value_from_ureq_response(resp)?)),
                    Err(ureq::Error::Status(_code, resp)) => {
                        Ok(Some(Self::value_from_ureq_response(resp)?))
                    }
                    Err(ureq::Error::Transport(e)) => {
                        let mut fields = HashMap::new();
                        fields.insert("status_code".to_string(), Value::Int(0));
                        fields.insert(
                            "status_text".to_string(),
                            Value::String("Transport Error".to_string()),
                        );
                        fields.insert("headers".to_string(), Value::Array(vec![]));
                        fields.insert("body".to_string(), Value::String(e.to_string()));
                        Ok(Some(Value::Struct("HttpResponse".to_string(), fields)))
                    }
                }
            }

            "tcp_listen" => {
                if args.len() != 2 {
                    return Err(format!(
                        "tcp_listen() expects 2 arguments, got {}",
                        args.len()
                    ));
                }
                let addr_val = self.eval_expression(&args[0])?;
                let _backlog_val = self.eval_expression(&args[1])?;
                let addr = match addr_val {
                    Value::String(s) => s,
                    _ => return Err("tcp_listen() address must be a string".to_string()),
                };
                let _ = _backlog_val;
                let listener =
                    TcpListener::bind(addr.as_str()).map_err(|e| format!("tcp_listen: {}", e))?;
                let id = self.alloc_tcp_id();
                self.tcp_listeners.insert(id, listener);
                Ok(Some(Value::TcpListener(id)))
            }

            "tcp_accept" => {
                if args.len() != 1 {
                    return Err(format!(
                        "tcp_accept() expects 1 argument, got {}",
                        args.len()
                    ));
                }
                let lid = match self.eval_expression(&args[0])? {
                    Value::TcpListener(id) => id,
                    _ => return Err("tcp_accept() requires a TcpListener".to_string()),
                };
                let listener = self
                    .tcp_listeners
                    .get_mut(&lid)
                    .ok_or_else(|| "tcp_accept: invalid TcpListener handle".to_string())?;
                let (stream, _addr) = listener
                    .accept()
                    .map_err(|e| format!("tcp_accept: {}", e))?;
                let sid = self.alloc_tcp_id();
                self.tcp_streams.insert(sid, stream);
                Ok(Some(Value::TcpStream(sid)))
            }

            "tcp_read" => {
                if args.len() != 2 {
                    return Err(format!(
                        "tcp_read() expects 2 arguments, got {}",
                        args.len()
                    ));
                }
                let sid = match self.eval_expression(&args[0])? {
                    Value::TcpStream(id) => id,
                    _ => return Err("tcp_read() requires a TcpStream".to_string()),
                };
                let max_bytes = match self.eval_expression(&args[1])? {
                    Value::Int(i) => i,
                    _ => return Err("tcp_read() max_bytes must be an int".to_string()),
                };
                if max_bytes <= 0 {
                    return Ok(Some(Value::String(String::new())));
                }
                let max_bytes = max_bytes as usize;
                let stream = self
                    .tcp_streams
                    .get_mut(&sid)
                    .ok_or_else(|| "tcp_read: invalid TcpStream handle".to_string())?;
                let mut buf = vec![0u8; max_bytes];
                let n = stream
                    .read(&mut buf)
                    .map_err(|e| format!("tcp_read: {}", e))?;
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                Ok(Some(Value::String(s)))
            }

            "tcp_write" => {
                if args.len() != 2 {
                    return Err(format!(
                        "tcp_write() expects 2 arguments, got {}",
                        args.len()
                    ));
                }
                let sid = match self.eval_expression(&args[0])? {
                    Value::TcpStream(id) => id,
                    _ => return Err("tcp_write() requires a TcpStream".to_string()),
                };
                let data = match self.eval_expression(&args[1])? {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                let stream = self
                    .tcp_streams
                    .get_mut(&sid)
                    .ok_or_else(|| "tcp_write: invalid TcpStream handle".to_string())?;
                let n = stream
                    .write(data.as_bytes())
                    .map_err(|e| format!("tcp_write: {}", e))?;
                stream
                    .flush()
                    .map_err(|e| format!("tcp_write: flush: {}", e))?;
                Ok(Some(Value::Int(n as i64)))
            }

            "tcp_close_stream" => {
                if args.len() != 1 {
                    return Err(format!(
                        "tcp_close_stream() expects 1 argument, got {}",
                        args.len()
                    ));
                }
                let sid = match self.eval_expression(&args[0])? {
                    Value::TcpStream(id) => id,
                    _ => return Err("tcp_close_stream() requires a TcpStream".to_string()),
                };
                if self.tcp_streams.remove(&sid).is_none() {
                    return Err("tcp_close_stream: invalid TcpStream handle".to_string());
                }
                Ok(Some(Value::Void))
            }

            "tcp_close_listener" => {
                if args.len() != 1 {
                    return Err(format!(
                        "tcp_close_listener() expects 1 argument, got {}",
                        args.len()
                    ));
                }
                let lid = match self.eval_expression(&args[0])? {
                    Value::TcpListener(id) => id,
                    _ => return Err("tcp_close_listener() requires a TcpListener".to_string()),
                };
                if self.tcp_listeners.remove(&lid).is_none() {
                    return Err("tcp_close_listener: invalid TcpListener handle".to_string());
                }
                Ok(Some(Value::Void))
            }

            "http_invoke_dispatch" => {
                if args.len() != 1 {
                    return Err(format!(
                        "http_invoke_dispatch() expects 1 argument, got {}",
                        args.len()
                    ));
                }
                let req = self.eval_expression(&args[0])?;
                match &req {
                    Value::Struct(name, _) if name == "Request" => {}
                    _ => {
                        return Err(format!(
                            "http_invoke_dispatch(req) requires a Request value, got {:?}",
                            req
                        ));
                    }
                }
                let result = self.call_function("dispatch", vec![req])?;
                match result {
                    Value::String(s) => Ok(Some(Value::String(s))),
                    other => Err(format!(
                        "dispatch() must return the full HTTP response as a string; got {:?}",
                        other
                    )),
                }
            }

            "dns_lookup" => {
                if args.len() != 1 {
                    return Err(format!(
                        "dns_lookup() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let host_val = self.eval_expression(&args[0])?;
                let host = match host_val {
                    Value::String(s) => s,
                    _ => return Err("dns_lookup() hostname must be a string".to_string()),
                };

                match (host.as_str(), 80u16).to_socket_addrs() {
                    Ok(mut iter) => {
                        let ip = iter.next().map(|a| a.ip().to_string()).unwrap_or_default();
                        Ok(Some(Value::String(ip)))
                    }
                    Err(_) => Ok(Some(Value::String(String::new()))),
                }
            }

            "reachable" => {
                if args.len() != 1 {
                    return Err(format!(
                        "reachable() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let host_val = self.eval_expression(&args[0])?;
                let host = match host_val {
                    Value::String(s) => s,
                    _ => return Err("reachable() hostname must be a string".to_string()),
                };

                let host = host.trim();
                if let Ok(addr) = host.parse::<SocketAddr>() {
                    return Ok(Some(Value::Bool(
                        TcpStream::connect_timeout(&addr, Duration::from_secs(4)).is_ok(),
                    )));
                }

                for port in [443u16, 80u16, 22u16] {
                    if let Ok(iter) = (host, port).to_socket_addrs() {
                        for addr in iter {
                            if TcpStream::connect_timeout(&addr, Duration::from_secs(4)).is_ok() {
                                return Ok(Some(Value::Bool(true)));
                            }
                        }
                    }
                }

                Ok(Some(Value::Bool(false)))
            }

            "enum_from_string" => {
                if args.len() != 2 {
                    return Err(format!(
                        "enum_from_string() expects 2 arguments, got {}",
                        args.len()
                    ));
                }

                let enum_name_value = self.eval_expression(&args[0])?;
                let variant_name_value = self.eval_expression(&args[1])?;

                if let (Value::String(enum_name), Value::String(variant_name)) =
                    (enum_name_value, variant_name_value)
                {
                    if let Some(variants) = self.enums.get(&enum_name) {
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
