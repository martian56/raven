use crate::ast::{ASTNode, Expression, Operator, Parameter};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Void,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::Bool(b) => write!(f, "{}", b),
            Value::String(s) => write!(f, "{}", s),
            Value::Void => write!(f, "void"),
        }
    }
}

#[derive(Clone)]
pub struct Function {
    params: Vec<Parameter>,
    body: ASTNode,
}

pub struct Interpreter {
    variables: HashMap<String, Value>,
    functions: HashMap<String, Function>,
    return_value: Option<Value>,
}

impl Interpreter {
    pub fn new() -> Self {
        Interpreter {
            variables: HashMap::new(),
            functions: HashMap::new(),
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

