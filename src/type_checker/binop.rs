use super::{Type, TypeChecker};
use crate::ast::{Expression, Operator};

impl TypeChecker {
    pub(super) fn check_binop(
        &mut self,
        left: &Expression,
        op: &Operator,
        right: &Expression,
    ) -> Result<Type, String> {
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
}
