use crate::ast::{ASTNode, Expression, Operator};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    Void,
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
            _ => Type::Unknown,
        }
    }
}

pub struct TypeChecker {
    // Symbol table: variable_name -> type
    variables: HashMap<String, Type>,
    // Function table: function_name -> (return_type, param_types)
    functions: HashMap<String, (Type, Vec<Type>)>,
}

impl TypeChecker {
    pub fn new() -> Self {
        TypeChecker {
            variables: HashMap::new(),
            functions: HashMap::new(),
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
                let declared_type = Type::from_string(type_str);
                let expr_type = self.check_expression(expr)?;

                if declared_type != expr_type {
                    return Err(format!(
                        "Type mismatch in variable '{}': expected {:?}, got {:?}",
                        name, declared_type, expr_type
                    ));
                }

                self.variables.insert(name.clone(), declared_type);
                Ok(Type::Void)
            }

            ASTNode::Assignment(name, expr) => {
                let expr_type = self.check_expression(expr)?;

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

            ASTNode::FunctionDecl(name, return_type_str, params, body) => {
                let return_type = Type::from_string(return_type_str);
                
                // Store parameter types in local scope
                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| Type::from_string(&p.param_type))
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
        }
    }

    fn check_expression(&mut self, expr: &Expression) -> Result<Type, String> {
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

            Expression::BinaryOp(left, op, right) => {
                let left_type = self.check_expression(left)?;
                let right_type = self.check_expression(right)?;

                match op {
                    Operator::Add | Operator::Subtract | Operator::Multiply | Operator::Divide => {
                        if left_type == Type::Int && right_type == Type::Int {
                            Ok(Type::Int)
                        } else if (left_type == Type::Float || left_type == Type::Int)
                            && (right_type == Type::Float || right_type == Type::Int)
                        {
                            Ok(Type::Float)
                        } else {
                            Err(format!(
                                "Type mismatch in arithmetic operation: {:?} {:?} {:?}",
                                left_type, op, right_type
                            ))
                        }
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

