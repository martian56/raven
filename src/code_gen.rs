use crate::ast::{ASTNode, Expression, Operator, Parameter};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<Value>), // Add proper array type
    Module(String), // Reference to a module by name
    Void,
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
            Value::Module(name) => write!(f, "<module: {}>", name),
            Value::Void => write!(f, "void"),
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
    pub exports: Vec<String>, // List of exported names
}

pub struct Interpreter {
    variables: HashMap<String, Value>,
    functions: HashMap<String, Function>,
    modules: HashMap<String, Module>, // module_name -> Module
    return_value: Option<Value>,
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter {
            variables: HashMap::new(),
            functions: HashMap::new(),
            modules: HashMap::new(),
            return_value: None,
        }
    }

    pub fn execute(&mut self, node: &ASTNode) -> Result<Value, String> {
        // Check if we have a return value set
        if self.return_value.is_some() {
            return Ok(self.return_value.clone().unwrap());
        }

        match node {
            ASTNode::VariableDecl(name, expr) => {
                let value = self.eval_expression(expr)?;
                self.variables.insert(name.clone(), value);
                Ok(Value::Void)
            }

            ASTNode::VariableDeclTyped(name, _type_str, expr) => {
                let value = self.eval_expression(expr)?;
                self.variables.insert(name.clone(), value);
                Ok(Value::Void)
            }

            ASTNode::Assignment(name, expr) => {
                let value = self.eval_expression(expr)?;
                if self.variables.contains_key(name) {
                    self.variables.insert(name.clone(), value);
                    Ok(Value::Void)
                } else {
                    Err(format!("Variable '{}' not declared", name))
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
                        
                        // Check for return in loop
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
                // Execute initialization
                self.execute(init)?;

                loop {
                    let cond_value = self.eval_expression(condition)?;

                    if let Value::Bool(true) = cond_value {
                        self.execute(body)?;
                        
                        // Check for return in loop
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
                    
                    // If we hit a return statement, stop executing
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
                // Check if this is a built-in function first
                if let Some(result) = self.call_builtin_function(name, args)? {
                    return Ok(result);
                }
                
                // Otherwise, call regular function
                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_expression(arg)?);
                }
                
                // Call the function
                self.call_function(name, evaluated_args)
            }
            
            ASTNode::MethodCall(object, method_name, args) => {
                // Evaluate all arguments
                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_expression(arg)?);
                }
                
                // For method calls as statements, we need to handle mutability
                if let Expression::Identifier(var_name) = object.as_ref() {
                    // This is a method call on a variable - we can mutate it
                    if let Some(Value::Array(mut elements)) = self.variables.get(var_name).cloned() {
                        match method_name.as_str() {
                            "push" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!("push() expects 1 argument, got {}", evaluated_args.len()));
                                }
                                elements.push(evaluated_args[0].clone());
                                self.variables.insert(var_name.clone(), Value::Array(elements));
                                Ok(Value::Void)
                            }
                            "pop" => {
                                if !evaluated_args.is_empty() {
                                    return Err(format!("pop() expects 0 arguments, got {}", evaluated_args.len()));
                                }
                                if elements.is_empty() {
                                    return Err("Cannot pop from empty array".to_string());
                                }
                                let popped = elements.pop().unwrap();
                                self.variables.insert(var_name.clone(), Value::Array(elements));
                                Ok(popped)
                            }
                            _ => {
                                // For other methods, use the expression evaluation
                                self.eval_expression(&Expression::MethodCall(object.clone(), method_name.clone(), args.clone()))
                            }
                        }
                    } else {
                        Err(format!("Variable '{}' is not an array", var_name))
                    }
                } else {
                    // For complex expressions, use the expression evaluation
                    self.eval_expression(&Expression::MethodCall(object.clone(), method_name.clone(), args.clone()))
                }
            }
            
            ASTNode::Import(module_name, alias) => {
                // Load the module
                self.load_module(module_name)?;
                
                // If there's an alias, create a reference to the module
                if let Some(alias_name) = alias {
                    self.variables.insert(alias_name.clone(), Value::Module(module_name.clone()));
                }
                
                Ok(Value::Void)
            }
            
            ASTNode::ImportSelective(module_name, items) => {
                // Load the module
                self.load_module(module_name)?;
                
                // Import specific items from the module
                if let Some(module) = self.modules.get(module_name) {
                    for item in items {
                        if let Some(value) = module.variables.get(item) {
                            self.variables.insert(item.clone(), value.clone());
                        } else if let Some(func) = module.functions.get(item) {
                            self.functions.insert(item.clone(), func.clone());
                        } else {
                            return Err(format!("Item '{}' not found in module '{}'", item, module_name));
                        }
                    }
                } else {
                    return Err(format!("Module '{}' not found", module_name));
                }
                
                Ok(Value::Void)
            }
            
            ASTNode::Export(stmt) => {
                // Execute the exported statement
                self.execute(stmt)
            }

            ASTNode::Return(expr) => {
                let value = self.eval_expression(expr)?;
                self.return_value = Some(value.clone());
                Ok(value)
            }
        }
    }

    fn eval_expression(&mut self, expr: &Expression) -> Result<Value, String> {
        match expr {
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

            Expression::BinaryOp(left, op, right) => {
                let left_val = self.eval_expression(left)?;
                let right_val = self.eval_expression(right)?;

                match (left_val, op, right_val) {
                    // Integer arithmetic
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

                    // Float arithmetic
                    (Value::Float(l), Operator::Add, Value::Float(r)) => Ok(Value::Float(l + r)),
                    (Value::Float(l), Operator::Subtract, Value::Float(r)) => Ok(Value::Float(l - r)),
                    (Value::Float(l), Operator::Multiply, Value::Float(r)) => Ok(Value::Float(l * r)),
                    (Value::Float(l), Operator::Divide, Value::Float(r)) => {
                        if r == 0.0 {
                            Err("Division by zero".to_string())
                        } else {
                            Ok(Value::Float(l / r))
                        }
                    }

                    // Mixed int/float arithmetic
                    (Value::Int(l), Operator::Add, Value::Float(r)) => Ok(Value::Float(l as f64 + r)),
                    (Value::Float(l), Operator::Add, Value::Int(r)) => Ok(Value::Float(l + r as f64)),
                    (Value::Int(l), Operator::Subtract, Value::Float(r)) => Ok(Value::Float(l as f64 - r)),
                    (Value::Float(l), Operator::Subtract, Value::Int(r)) => Ok(Value::Float(l - r as f64)),
                    (Value::Int(l), Operator::Multiply, Value::Float(r)) => Ok(Value::Float(l as f64 * r)),
                    (Value::Float(l), Operator::Multiply, Value::Int(r)) => Ok(Value::Float(l * r as f64)),
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

                    // String concatenation
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

                    // Integer comparisons
                    (Value::Int(l), Operator::Equal, Value::Int(r)) => Ok(Value::Bool(l == r)),
                    (Value::Int(l), Operator::NotEqual, Value::Int(r)) => Ok(Value::Bool(l != r)),
                    (Value::Int(l), Operator::LessThan, Value::Int(r)) => Ok(Value::Bool(l < r)),
                    (Value::Int(l), Operator::GreaterThan, Value::Int(r)) => Ok(Value::Bool(l > r)),
                    (Value::Int(l), Operator::LessEqual, Value::Int(r)) => Ok(Value::Bool(l <= r)),
                    (Value::Int(l), Operator::GreaterEqual, Value::Int(r)) => Ok(Value::Bool(l >= r)),

                    // Float comparisons
                    (Value::Float(l), Operator::Equal, Value::Float(r)) => Ok(Value::Bool(l == r)),
                    (Value::Float(l), Operator::NotEqual, Value::Float(r)) => Ok(Value::Bool(l != r)),
                    (Value::Float(l), Operator::LessThan, Value::Float(r)) => Ok(Value::Bool(l < r)),
                    (Value::Float(l), Operator::GreaterThan, Value::Float(r)) => Ok(Value::Bool(l > r)),
                    (Value::Float(l), Operator::LessEqual, Value::Float(r)) => Ok(Value::Bool(l <= r)),
                    (Value::Float(l), Operator::GreaterEqual, Value::Float(r)) => Ok(Value::Bool(l >= r)),

                    // Boolean operations
                    (Value::Bool(l), Operator::And, Value::Bool(r)) => Ok(Value::Bool(l && r)),
                    (Value::Bool(l), Operator::Or, Value::Bool(r)) => Ok(Value::Bool(l || r)),
                    (Value::Bool(l), Operator::Equal, Value::Bool(r)) => Ok(Value::Bool(l == r)),
                    (Value::Bool(l), Operator::NotEqual, Value::Bool(r)) => Ok(Value::Bool(l != r)),

                    // String comparisons
                    (Value::String(l), Operator::Equal, Value::String(r)) => Ok(Value::Bool(l == r)),
                    (Value::String(l), Operator::NotEqual, Value::String(r)) => Ok(Value::Bool(l != r)),

                    _ => Err(format!(
                        "Type error in binary operation: {:?} {:?}",
                        left, right
                    )),
                }
            }

            Expression::FunctionCall(name, args) => {
                // Check if this is a built-in function first
                if let Some(result) = self.call_builtin_function(name, args)? {
                    return Ok(result);
                }
                
                // Otherwise, call regular function
                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_expression(arg)?);
                }
                
                // Call the function
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
                                index_int, elements.len()
                            ));
                        }
                        Ok(elements[index_int as usize].clone())
                    }
                    Value::String(s) => {
                        if index_int < 0 || index_int as usize >= s.len() {
                            return Err(format!(
                                "String index {} out of bounds (string length: {})",
                                index_int, s.len()
                            ));
                        }
                        let ch = s.chars().nth(index_int as usize)
                            .ok_or_else(|| "Invalid character index".to_string())?;
                        Ok(Value::String(ch.to_string()))
                    }
                    _ => Err("Cannot index non-array or non-string value".to_string()),
                }
            }
            
            Expression::MethodCall(object_expr, method_name, args) => {
                // Evaluate arguments
                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_expression(arg)?);
                }
                
                // Check if this is a method call on a variable (for mutability)
                if let Expression::Identifier(var_name) = object_expr.as_ref() {
                    if let Some(Value::Array(mut elements)) = self.variables.get(var_name).cloned() {
                        match method_name.as_str() {
                            "push" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!("push() expects 1 argument, got {}", evaluated_args.len()));
                                }
                                elements.push(evaluated_args[0].clone());
                                self.variables.insert(var_name.clone(), Value::Array(elements.clone()));
                                Ok(Value::Array(elements))
                            }
                            "pop" => {
                                if !evaluated_args.is_empty() {
                                    return Err(format!("pop() expects 0 arguments, got {}", evaluated_args.len()));
                                }
                                if elements.is_empty() {
                                    return Err("Cannot pop from empty array".to_string());
                                }
                                let popped = elements.pop().unwrap();
                                self.variables.insert(var_name.clone(), Value::Array(elements));
                                Ok(popped)
                            }
                            "slice" => {
                                if evaluated_args.len() != 2 {
                                    return Err(format!("slice() expects 2 arguments, got {}", evaluated_args.len()));
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
                                    return Err(format!("join() expects 1 argument, got {}", evaluated_args.len()));
                                }
                                let delimiter = match &evaluated_args[0] {
                                    Value::String(d) => d,
                                    _ => return Err("join() delimiter must be string".to_string()),
                                };
                                
                                let strings: Vec<String> = elements.iter()
                                    .map(|v| v.to_string())
                                    .collect();
                                
                                Ok(Value::String(strings.join(delimiter)))
                            }
                            _ => Err(format!("Unknown method '{}' for array", method_name)),
                        }
                    } else if let Some(Value::Module(module_name)) = self.variables.get(var_name) {
                        // Handle module method calls
                        let module_name_clone = module_name.clone();
                        if let Some(module) = self.modules.get(&module_name_clone) {
                            if let Some(func) = module.functions.get(method_name) {
                                // Clone the function to avoid borrow conflicts
                                let func_clone = func.clone();
                                let module_clone = module.clone();
                                // Call the function from the module
                                self.call_function_with_module(&func_clone, evaluated_args, &module_clone)
                            } else if let Some(value) = module.variables.get(method_name) {
                                // Return the variable from the module
                                Ok(value.clone())
                            } else {
                                Err(format!("Method '{}' not found in module '{}'", method_name, module_name))
                            }
                        } else {
                            Err(format!("Module '{}' not found", module_name))
                        }
                    } else if let Some(Value::String(s)) = self.variables.get(var_name) {
                        // Handle string method calls (strings are immutable, so we don't update the variable)
                        match method_name.as_str() {
                            "slice" => {
                                if evaluated_args.len() != 2 {
                                    return Err(format!("slice() expects 2 arguments, got {}", evaluated_args.len()));
                                }
                                let start = match &evaluated_args[0] {
                                    Value::Int(i) => *i,
                                    _ => return Err("slice() start index must be integer".to_string()),
                                };
                                let end = match &evaluated_args[1] {
                                    Value::Int(i) => *i,
                                    _ => return Err("slice() end index must be integer".to_string()),
                                };
                                
                                if start < 0 || end < 0 || start > end || start as usize >= s.len() {
                                    return Err("Invalid slice indices".to_string());
                                }
                                
                                let start_idx = start as usize;
                                let end_idx = (end as usize).min(s.len());
                                
                                Ok(Value::String(s[start_idx..end_idx].to_string()))
                            }
                            "split" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!("split() expects 1 argument, got {}", evaluated_args.len()));
                                }
                                let delimiter = match &evaluated_args[0] {
                                    Value::String(d) => d,
                                    _ => return Err("split() delimiter must be string".to_string()),
                                };
                                
                                let parts: Vec<Value> = s.split(delimiter)
                                    .map(|part| Value::String(part.to_string()))
                                    .collect();
                                
                                Ok(Value::Array(parts))
                            }
                            "replace" => {
                                if evaluated_args.len() != 2 {
                                    return Err(format!("replace() expects 2 arguments, got {}", evaluated_args.len()));
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
                            _ => Err(format!("Unknown method '{}' for string", method_name)),
                        }
                    } else {
                        Err(format!("Variable '{}' is not an array, module, or string", var_name))
                    }
                } else {
                    // For complex expressions, evaluate normally without mutability
                    let object = self.eval_expression(object_expr)?;
                    
                    if let Value::Array(elements) = object {
                        match method_name.as_str() {
                            "push" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!("push() expects 1 argument, got {}", evaluated_args.len()));
                                }
                                let mut new_elements = elements.clone();
                                new_elements.push(evaluated_args[0].clone());
                                Ok(Value::Array(new_elements))
                            }
                            "pop" => {
                                if !evaluated_args.is_empty() {
                                    return Err(format!("pop() expects 0 arguments, got {}", evaluated_args.len()));
                                }
                                if elements.is_empty() {
                                    return Err("Cannot pop from empty array".to_string());
                                }
                                Ok(elements.last().unwrap().clone())
                            }
                            "slice" => {
                                if evaluated_args.len() != 2 {
                                    return Err(format!("slice() expects 2 arguments, got {}", evaluated_args.len()));
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
                                    return Err(format!("join() expects 1 argument, got {}", evaluated_args.len()));
                                }
                                let delimiter = match &evaluated_args[0] {
                                    Value::String(d) => d,
                                    _ => return Err("join() delimiter must be string".to_string()),
                                };
                                
                                let strings: Vec<String> = elements.iter()
                                    .map(|v| v.to_string())
                                    .collect();
                                
                                Ok(Value::String(strings.join(delimiter)))
                            }
                            _ => Err(format!("Unknown method '{}' for array", method_name)),
                        }
                    } else if let Value::String(s) = object {
                        // Handle string methods
                        match method_name.as_str() {
                            "slice" => {
                                if evaluated_args.len() != 2 {
                                    return Err(format!("slice() expects 2 arguments, got {}", evaluated_args.len()));
                                }
                                let start = match &evaluated_args[0] {
                                    Value::Int(i) => *i,
                                    _ => return Err("slice() start index must be integer".to_string()),
                                };
                                let end = match &evaluated_args[1] {
                                    Value::Int(i) => *i,
                                    _ => return Err("slice() end index must be integer".to_string()),
                                };
                                
                                if start < 0 || end < 0 || start > end || start as usize >= s.len() {
                                    return Err("Invalid slice indices".to_string());
                                }
                                
                                let start_idx = start as usize;
                                let end_idx = (end as usize).min(s.len());
                                
                                Ok(Value::String(s[start_idx..end_idx].to_string()))
                            }
                            "split" => {
                                if evaluated_args.len() != 1 {
                                    return Err(format!("split() expects 1 argument, got {}", evaluated_args.len()));
                                }
                                let delimiter = match &evaluated_args[0] {
                                    Value::String(d) => d,
                                    _ => return Err("split() delimiter must be string".to_string()),
                                };
                                
                                let parts: Vec<Value> = s.split(delimiter)
                                    .map(|part| Value::String(part.to_string()))
                                    .collect();
                                
                                Ok(Value::Array(parts))
                            }
                            "replace" => {
                                if evaluated_args.len() != 2 {
                                    return Err(format!("replace() expects 2 arguments, got {}", evaluated_args.len()));
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
                            _ => Err(format!("Unknown method '{}' for string", method_name)),
                        }
                    } else {
                        Err(format!("Cannot call methods on non-array or non-string value of type {:?}", object))
                    }
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

            // Save current variables (for scope)
            let saved_vars = self.variables.clone();

            // Bind parameters
            for (i, param) in func.params.iter().enumerate() {
                self.variables.insert(param.name.clone(), args[i].clone());
            }

            // Execute function body
            self.return_value = None;
            self.execute(&func.body)?;

            // Get return value
            let result = self.return_value.clone().unwrap_or(Value::Void);
            self.return_value = None;

            // Restore variables
            self.variables = saved_vars;

            Ok(result)
        } else {
            Err(format!("Function '{}' not found", name))
        }
    }

    fn call_builtin_function(&mut self, name: &str, args: &[Expression]) -> Result<Option<Value>, String> {
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
                    Value::Int(_) => "int",
                    Value::Float(_) => "float",
                    Value::Bool(_) => "bool",
                    Value::String(_) => "string",
                    Value::Array(_) => "array",
                    Value::Module(_) => "module",
                    Value::Void => "void",
                };
                Ok(Some(Value::String(type_name.to_string())))
            }
            
            "print" => {
                if args.is_empty() {
                    return Err("print() expects at least 1 argument".to_string());
                }
                
                // Handle formatted print with placeholders
                if args.len() == 1 {
                    // Simple print: print(value)
                    let value = self.eval_expression(&args[0])?;
                    println!("{}", value);
                } else {
                    // Formatted print: print(format_string, arg1, arg2, ...)
                    let format_string = self.eval_expression(&args[0])?;
                    if let Value::String(format_str) = format_string {
                        let mut formatted = format_str.clone();
                        
                        // Replace {} placeholders with arguments
                        for i in 1..args.len() {
                            let arg_value = self.eval_expression(&args[i])?;
                            let placeholder = "{}";
                            if let Some(pos) = formatted.find(placeholder) {
                                formatted.replace_range(pos..pos + placeholder.len(), &arg_value.to_string());
                            } else {
                                return Err(format!("Too many arguments for print() - format string has no placeholder for argument {}", i));
                            }
                        }
                        
                        // Check if there are any remaining placeholders
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
            
            "input" => {
                use std::io::{self, Write};
                
                if args.len() > 1 {
                    return Err(format!("input() expects 0 or 1 argument, got {}", args.len()));
                }
                
                // Print prompt if provided
                if args.len() == 1 {
                    let prompt = self.eval_expression(&args[0])?;
                    if let Value::String(prompt_str) = prompt {
                        print!("{}", prompt_str);
                        io::stdout().flush().unwrap();
                    } else {
                        return Err("input() prompt must be a string".to_string());
                    }
                }
                
                // Read user input
                let mut input = String::new();
                match io::stdin().read_line(&mut input) {
                    Ok(_) => {
                        // Remove trailing newline
                        input = input.trim().to_string();
                        Ok(Some(Value::String(input)))
                    }
                    Err(e) => Err(format!("Error reading input: {}", e)),
                }
            }
            
            "read_file" => {
                if args.len() != 1 {
                    return Err(format!("read_file() expects 1 argument, got {}", args.len()));
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
                    return Err(format!("write_file() expects 2 arguments, got {}", args.len()));
                }
                
                let filename = self.eval_expression(&args[0])?;
                let content = self.eval_expression(&args[1])?;
                
                if let Value::String(filename_str) = filename {
                    let content_str = match content {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    
                    // Convert literal \n to actual newlines
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
                    return Err(format!("append_file() expects 2 arguments, got {}", args.len()));
                }
                
                let filename = self.eval_expression(&args[0])?;
                let content = self.eval_expression(&args[1])?;
                
                if let Value::String(filename_str) = filename {
                    let content_str = match content {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    
                    // Convert literal \n to actual newlines
                    let processed_content = content_str.replace("\\n", "\n");
                    
                    match fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&filename_str) {
                        Ok(mut file) => {
                            use std::io::Write;
                            match file.write_all(processed_content.as_bytes()) {
                                Ok(_) => Ok(Some(Value::Void)),
                                Err(e) => Err(format!("Error appending to file '{}': {}", filename_str, e)),
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
                    return Err(format!("file_exists() expects 1 argument, got {}", args.len()));
                }
                
                let filename = self.eval_expression(&args[0])?;
                if let Value::String(filename_str) = filename {
                    let exists = Path::new(&filename_str).exists();
                    Ok(Some(Value::Bool(exists)))
                } else {
                    Err("file_exists() filename must be a string".to_string())
                }
            }
            
            "format" => {
                if args.len() < 1 {
                    return Err(format!("format() expects at least 1 argument, got {}", args.len()));
                }
                
                let template = self.eval_expression(&args[0])?;
                if let Value::String(template_str) = template {
                    let mut result = template_str.clone();
                    let mut arg_index = 1;
                    
                    // Replace {} placeholders with arguments
                    while let Some(pos) = result.find("{}") {
                        if arg_index >= args.len() {
                            return Err("format() not enough arguments for placeholders".to_string());
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
            
            _ => Ok(None), // Not a built-in function
        }
    }
    
    fn load_module(&mut self, module_name: &str) -> Result<(), String> {
        // Check if module is already loaded
        if self.modules.contains_key(module_name) {
            return Ok(());
        }
        
        // Load module file
        let module_path = if module_name.ends_with(".rv") {
            module_name.to_string()
        } else {
            format!("{}.rv", module_name)
        };
        
        let content = fs::read_to_string(&module_path)
            .map_err(|e| format!("Failed to load module '{}': {}", module_path, e))?;
        
        // Parse the module
        let lexer = crate::lexer::Lexer::new(content.clone());
        let mut parser = crate::parser::Parser::new(lexer, content);
        let ast = parser.parse()
            .map_err(|e| format!("Failed to parse module '{}': {}", module_path, e.format()))?;
        
        // Create a new interpreter for the module
        let mut module_interpreter = Interpreter::new();
        
        // Execute the module to populate its exports
        module_interpreter.execute(&ast)?;
        
        // Extract exports from the module
        let mut module = Module {
            variables: module_interpreter.variables,
            functions: module_interpreter.functions,
            exports: Vec::new(),
        };
        
        // TODO: Track exports properly during execution
        // For now, we'll assume all variables and functions are exported
        
        // Store the module
        self.modules.insert(module_name.to_string(), module);
        
        Ok(())
    }
    
    fn call_function_with_module(&mut self, func: &Function, args: Vec<Value>, module: &Module) -> Result<Value, String> {
        // Create a new scope for the function call
        let mut function_variables = HashMap::new();
        
        // Add module variables to the function scope
        for (name, value) in &module.variables {
            function_variables.insert(name.clone(), value.clone());
        }
        
        // Add function parameters to the scope
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
        
        // Save current variables and set function variables
        let old_variables = std::mem::replace(&mut self.variables, function_variables);
        let old_return_value = self.return_value.take();
        
        // Execute function body
        let result = self.execute(&func.body);
        
        // Restore old variables and return value
        self.variables = old_variables;
        self.return_value = old_return_value;
        
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Expression;

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
        let node = ASTNode::Print(Box::new(Expression::StringLiteral("Hello, Raven!".to_string())));

        assert!(interp.execute(&node).is_ok());
    }
}

