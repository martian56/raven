use crate::ast::{ASTNode, Expression, Operator, Parameter};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};

mod array_ops;
mod binop;
mod builtins;
mod calls;
mod methods;
mod modules;
mod stmt;
mod value;
pub use value::Value;

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

            Expression::BinaryOp(left, op, right) => self.eval_binop(left, op, right),

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
}

#[cfg(test)]
mod tests;
