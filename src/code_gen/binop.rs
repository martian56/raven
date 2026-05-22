use super::{Interpreter, Value};
use crate::ast::{Expression, Operator};

impl Interpreter {
    pub(super) fn eval_binop(
        &mut self,
        left: &Expression,
        op: &Operator,
        right: &Expression,
    ) -> Result<Value, String> {
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
            (Value::Float(l), Operator::Subtract, Value::Float(r)) => Ok(Value::Float(l - r)),
            (Value::Float(l), Operator::Multiply, Value::Float(r)) => Ok(Value::Float(l * r)),
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
            (Value::Int(l), Operator::GreaterEqual, Value::Int(r)) => Ok(Value::Bool(l >= r)),

            (Value::Float(l), Operator::Equal, Value::Float(r)) => Ok(Value::Bool(l == r)),
            (Value::Float(l), Operator::NotEqual, Value::Float(r)) => Ok(Value::Bool(l != r)),
            (Value::Float(l), Operator::LessThan, Value::Float(r)) => Ok(Value::Bool(l < r)),
            (Value::Float(l), Operator::GreaterThan, Value::Float(r)) => Ok(Value::Bool(l > r)),
            (Value::Float(l), Operator::LessEqual, Value::Float(r)) => Ok(Value::Bool(l <= r)),
            (Value::Float(l), Operator::GreaterEqual, Value::Float(r)) => Ok(Value::Bool(l >= r)),

            (Value::Bool(l), Operator::And, Value::Bool(r)) => Ok(Value::Bool(l && r)),
            (Value::Bool(l), Operator::Or, Value::Bool(r)) => Ok(Value::Bool(l || r)),
            (Value::Bool(l), Operator::Equal, Value::Bool(r)) => Ok(Value::Bool(l == r)),
            (Value::Bool(l), Operator::NotEqual, Value::Bool(r)) => Ok(Value::Bool(l != r)),

            (Value::String(l), Operator::Equal, Value::String(r)) => Ok(Value::Bool(l == r)),
            (Value::String(l), Operator::NotEqual, Value::String(r)) => Ok(Value::Bool(l != r)),

            (Value::TcpListener(l), Operator::Equal, Value::TcpListener(r)) => {
                Ok(Value::Bool(l == r))
            }
            (Value::TcpListener(l), Operator::NotEqual, Value::TcpListener(r)) => {
                Ok(Value::Bool(l != r))
            }
            (Value::TcpStream(l), Operator::Equal, Value::TcpStream(r)) => Ok(Value::Bool(l == r)),
            (Value::TcpStream(l), Operator::NotEqual, Value::TcpStream(r)) => {
                Ok(Value::Bool(l != r))
            }

            _ => Err(format!(
                "Type error in binary operation: {:?} {:?}",
                left, right
            )),
        }
    }
}
