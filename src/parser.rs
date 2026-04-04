#![allow(clippy::result_large_err)]

use crate::ast::{
    ASTNode, EnumMember, Expression, ImplMember, Operator, StructMember,
};
use crate::error::{parse_error, RavenError};
use crate::lexer::{Lexer, TokenType};
use crate::span::Span;

pub struct Parser {
    lexer: Lexer,
    current_token: Option<TokenType>,
    source_code: String,
}

impl Parser {
    pub fn new(mut lexer: Lexer, source_code: String) -> Self {
        let first_token = lexer.next_token();
        Parser {
            lexer,
            current_token: Some(first_token),
            source_code,
        }
    }

    fn advance(&mut self) {
        self.current_token = Some(self.lexer.next_token());
    }

    pub fn parse(&mut self) -> Result<ASTNode, RavenError> {
        let mut statements: Vec<ASTNode> = Vec::new();

        while let Some(token) = &self.current_token {
            let stmt: ASTNode = match token {
                TokenType::Let => self.parse_variable_declaration()?,
                TokenType::Struct => self.parse_struct_declaration()?,
                TokenType::Impl => self.parse_impl_block()?,
                TokenType::Enum => self.parse_enum_declaration()?,
                TokenType::Identifier(_) => {
                    let expr = self.parse_expression()?;

                    if let Some(TokenType::Assign) = &self.current_token {
                        self.advance();
                        let value_expr = self.parse_expression()?;

                        if let Some(TokenType::Semicolon) = &self.current_token {
                            self.advance();
                        } else {
                            let span = Span::new(
                                self.lexer.line,
                                self.lexer.column,
                                self.lexer.position,
                                1,
                            );
                            return Err(parse_error("Expected ';' after assignment", span)
                                .with_source(self.source_code.clone())
                                .with_hint("Add ';' at the end".to_string()));
                        }

                        ASTNode::Assignment(Box::new(expr), Box::new(value_expr))
                    } else if let Some(TokenType::Semicolon) = &self.current_token {
                        self.advance();
                        ASTNode::ExpressionStatement(expr)
                    } else {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ';' or '=' after expression", span)
                            .with_source(self.source_code.clone())
                            .with_hint(
                                "Add ';' for expression statement or '=' for assignment"
                                    .to_string(),
                            ));
                    }
                }
                TokenType::If => self.parse_if_statement()?,
                TokenType::While => self.parse_while_loop()?,
                TokenType::For => self.parse_for_loop()?,
                TokenType::Fun => self.parse_function_declaration()?,
                TokenType::Return => self.parse_return_statement()?,
                TokenType::Print => self.parse_print_statement()?,
                TokenType::Import => self.parse_import_statement()?,
                TokenType::Export => self.parse_export_statement()?,
                TokenType::Comment(text) => {
                    let text = text.clone();
                    self.advance();
                    ASTNode::Comment(text)
                }
                TokenType::EOF => break,
                _ => {
                    let span =
                        Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(parse_error(
                        format!("Unexpected token: {:?}", token),
                        span,
                    )
                    .with_source(self.source_code.clone())
                    .with_hint("Expected a statement: let, fun, struct, impl, if, while, for, print, import, or identifier.".to_string()));
                }
            };

            statements.push(stmt);
        }

        Ok(ASTNode::Block(statements))
    }

    /// Parses a type: primitive, `void`, or identifier, followed by any number of `[]` dimensions.
    /// Examples: `int`, `int[][]`, `Point`, `Point[]`, `string[][][]`.
    fn parse_type_string(&mut self) -> Result<String, RavenError> {
        let mut s = match &self.current_token {
            Some(TokenType::IntType) => {
                self.advance();
                "int".to_string()
            }
            Some(TokenType::FloatType) => {
                self.advance();
                "float".to_string()
            }
            Some(TokenType::BoolType) => {
                self.advance();
                "bool".to_string()
            }
            Some(TokenType::StringType) => {
                self.advance();
                "string".to_string()
            }
            Some(TokenType::VoidType) => {
                self.advance();
                "void".to_string()
            }
            Some(TokenType::Identifier(name)) => {
                let n = name.clone();
                self.advance();
                n
            }
            _ => {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected type", span)
                        .with_source(self.source_code.clone())
                        .with_hint(
                            "Use: int, float, bool, string, void, or a struct/enum name, with optional []"
                                .to_string(),
                        ),
                );
            }
        };

        while let Some(TokenType::LeftBracket) = &self.current_token {
            self.advance();
            if let Some(TokenType::RightBracket) = &self.current_token {
                self.advance();
                s.push_str("[]");
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ']' after '[' in array type", span)
                    .with_source(self.source_code.clone())
                    .with_hint(
                        "Close each dimension with ]; e.g. int[][], Point[][]".to_string(),
                    ));
            }
        }

        Ok(s)
    }

    fn parse_variable_declaration(&mut self) -> Result<ASTNode, RavenError> {
        self.advance();

        let var_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone = name.clone();
            self.advance();
            name_clone
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected identifier after 'let'", span)
                .with_source(self.source_code.clone())
                .with_hint("Use: let name: type = value;".to_string()));
        };

        if let Some(TokenType::Colon) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected ':' after variable name", span)
                .with_source(self.source_code.clone())
                .with_hint("Use: let name: type = value;".to_string()));
        }

        let var_type = self.parse_type_string()?;

        if let Some(TokenType::Assign) = &self.current_token {
            self.advance();

            let expr_start_line = self.lexer.line;
            let expr = self.parse_expression()?;

            if let Some(TokenType::Semicolon) = &self.current_token {
                self.advance();
            } else {
                let error_line = if self.lexer.line > expr_start_line {
                    expr_start_line
                } else {
                    self.lexer.line
                };

                let lines: Vec<&str> = self.source_code.lines().collect();
                let error_column = if error_line < lines.len() {
                    lines[error_line].len()
                } else {
                    self.lexer.column
                };

                let span = Span::new(error_line, error_column, self.lexer.position, 1);
                return Err(parse_error("Expected ';' after variable declaration", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add ';' at the end of the statement".to_string()));
            }

            Ok(ASTNode::VariableDeclTyped(
                var_name,
                var_type,
                Box::new(expr),
            ))
        } else if let Some(TokenType::Semicolon) = &self.current_token {
            self.advance();

            let default_expr = match var_type.as_str() {
                "int" => Expression::Integer(0),
                "float" => Expression::Float(0.0),
                "bool" => Expression::Boolean(false),
                "string" => Expression::StringLiteral("".to_string()),
                _ if var_type.ends_with("[]") => Expression::ArrayLiteral(vec![]),
                _ => Expression::Uninitialized,
            };

            Ok(ASTNode::VariableDeclTyped(
                var_name,
                var_type,
                Box::new(default_expr),
            ))
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '=' or ';' after variable type", span)
                .with_source(self.source_code.clone())
                .with_hint(
                    "Add '=' with initial value or ';' for declaration only. For array types use int[], string[], etc."
                        .to_string(),
                ));
        }
    }

    fn parse_if_statement(&mut self) -> Result<ASTNode, RavenError> {
        self.parse_if_like_statement(true)
    }

    fn parse_if_like_statement(&mut self, is_if: bool) -> Result<ASTNode, RavenError> {
        self.advance();

        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance();
            let condition: Expression = self.parse_expression()?;

            if let Some(TokenType::RightParen) = &self.current_token {
                self.advance();

                if let Some(TokenType::LeftBrace) = &self.current_token {
                    self.advance();
                    let body: ASTNode = self.parse_block()?;
                    self.advance();

                    let mut else_if = None;
                    let mut else_block = None;

                    if let Some(token) = &self.current_token {
                        match token {
                            TokenType::ElseIf => {
                                else_if = Some(self.parse_if_like_statement(false)?);
                            }
                            TokenType::Else => {
                                self.advance();
                                if let Some(TokenType::LeftBrace) = &self.current_token {
                                    self.advance();
                                    else_block = Some(self.parse_block()?);
                                    self.advance();
                                } else {
                                    let span = Span::new(
                                        self.lexer.line,
                                        self.lexer.column,
                                        self.lexer.position,
                                        1,
                                    );
                                    return Err(parse_error("Expected '{' after 'else'", span)
                                        .with_source(self.source_code.clone())
                                        .with_hint("Add '{' to start the else block".to_string()));
                                }
                            }
                            _ => {}
                        }
                    }

                    return Ok(ASTNode::IfStatement(
                        Box::new(condition),
                        Box::new(body),
                        else_if.map(Box::new),
                        else_block.map(Box::new),
                    ));
                } else {
                    let span =
                        Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(parse_error("Expected '{' to start if block", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Add '{' after the condition".to_string()));
                }
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ')' after condition", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Close the condition with ')'".to_string()));
            }
        }

        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
        let keyword = if is_if { "if" } else { "elseif" };
        Err(
            parse_error(format!("Expected '(' after '{}'", keyword), span)
                .with_source(self.source_code.clone())
                .with_hint(format!("Use: {} (condition) {{ ... }}", keyword)),
        )
    }

    fn parse_block(&mut self) -> Result<ASTNode, RavenError> {
        let mut statements = Vec::new();

        while let Some(token) = &self.current_token {
            if let TokenType::RightBrace = token {
                break;
            }

            let stmt = match token {
                TokenType::Let => self.parse_variable_declaration()?,
                TokenType::Identifier(_) => {
                    let expr = self.parse_expression()?;

                    if let Some(TokenType::Assign) = &self.current_token {
                        self.advance();
                        let value_expr = self.parse_expression()?;

                        if let Some(TokenType::Semicolon) = &self.current_token {
                            self.advance();
                        } else {
                            let span = Span::new(
                                self.lexer.line,
                                self.lexer.column,
                                self.lexer.position,
                                1,
                            );
                            return Err(parse_error("Expected ';' after assignment", span)
                                .with_source(self.source_code.clone())
                                .with_hint("Add ';' at the end".to_string()));
                        }

                        ASTNode::Assignment(Box::new(expr), Box::new(value_expr))
                    } else if let Some(TokenType::Semicolon) = &self.current_token {
                        self.advance();
                        ASTNode::ExpressionStatement(expr)
                    } else {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ';' or '=' after expression", span)
                            .with_source(self.source_code.clone())
                            .with_hint(
                                "Add ';' for expression statement or '=' for assignment"
                                    .to_string(),
                            ));
                    }
                }
                TokenType::If => self.parse_if_statement()?,
                TokenType::While => self.parse_while_loop()?,
                TokenType::For => self.parse_for_loop()?,
                TokenType::Return => self.parse_return_statement()?,
                TokenType::Print => self.parse_print_statement()?,
                TokenType::Comment(text) => {
                    let text = text.clone();
                    self.advance();
                    ASTNode::Comment(text)
                }
                _ => {
                    let span =
                        Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(parse_error(
                        format!("Unexpected token in block: {:?}", token),
                        span,
                    )
                    .with_source(self.source_code.clone())
                    .with_hint("Expected a statement or '}}' to close the block.".to_string()));
                }
            };

            statements.push(stmt);
        }

        Ok(ASTNode::Block(statements))
    }

    fn parse_expression(&mut self) -> Result<Expression, RavenError> {
        self.parse_expression_with_precedence(0)
    }

    fn parse_expression_with_precedence(
        &mut self,
        min_precedence: u8,
    ) -> Result<Expression, RavenError> {
        let mut left = self.parse_term()?;

        while let Some(op) = self.match_operator() {
            let precedence = self.operator_precedence(&op);

            if precedence < min_precedence {
                break;
            }

            self.advance();

            let right = self.parse_expression_with_precedence(precedence + 1)?;

            left = Expression::BinaryOp(Box::new(left), op, Box::new(right));
        }

        Ok(left)
    }

    fn operator_precedence(&self, op: &Operator) -> u8 {
        match op {
            Operator::Or => 1,
            Operator::And => 2,
            Operator::Equal | Operator::NotEqual => 3,
            Operator::LessThan
            | Operator::GreaterThan
            | Operator::LessEqual
            | Operator::GreaterEqual => 4,
            Operator::Add | Operator::Subtract => 5,
            Operator::Multiply | Operator::Divide | Operator::Modulo => 6,
            Operator::UnaryMinus | Operator::Not => 7,
        }
    }

    fn match_operator(&self) -> Option<Operator> {
        match self.current_token {
            Some(TokenType::Plus) => Some(Operator::Add),
            Some(TokenType::Minus) => Some(Operator::Subtract),
            Some(TokenType::Star) => Some(Operator::Multiply),
            Some(TokenType::Slash) => Some(Operator::Divide),
            Some(TokenType::Percent) => Some(Operator::Modulo),
            Some(TokenType::EqualEqual) => Some(Operator::Equal),
            Some(TokenType::Less) => Some(Operator::LessThan),
            Some(TokenType::Greater) => Some(Operator::GreaterThan),
            Some(TokenType::LessEqual) => Some(Operator::LessEqual),
            Some(TokenType::GreaterEqual) => Some(Operator::GreaterEqual),
            Some(TokenType::NotEqual) => Some(Operator::NotEqual),
            Some(TokenType::And) => Some(Operator::And),
            Some(TokenType::Or) => Some(Operator::Or),
            _ => None,
        }
    }

    fn parse_term(&mut self) -> Result<Expression, RavenError> {
        match &self.current_token {
            Some(TokenType::Minus) => {
                self.advance();
                let expr = self.parse_term()?;
                Ok(Expression::UnaryOp(Operator::UnaryMinus, Box::new(expr)))
            }
            Some(TokenType::Not) => {
                self.advance();
                let expr = self.parse_term()?;
                Ok(Expression::UnaryOp(Operator::Not, Box::new(expr)))
            }
            Some(TokenType::IntLiteral(value)) => {
                let val = *value;
                self.advance();
                Ok(Expression::Integer(val))
            }
            Some(TokenType::FloatLiteral(value)) => {
                let val = *value;
                self.advance();
                Ok(Expression::Float(val))
            }
            Some(TokenType::BoolLiteral(value)) => {
                let val = *value;
                self.advance();
                Ok(Expression::Boolean(val))
            }
            Some(TokenType::StringLiteral(s)) => {
                let s_clone = s.clone();
                self.advance();
                Ok(Expression::StringLiteral(s_clone))
            }
            Some(TokenType::LeftParen) => {
                self.advance();
                let expr = self.parse_expression()?;
                if let Some(TokenType::RightParen) = &self.current_token {
                    self.advance();
                } else {
                    let span =
                        Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(parse_error("Expected ')' to close parenthesized expression", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Add ')' after the expression".to_string()));
                }
                Ok(expr)
            }
            Some(TokenType::LeftBracket) => self.parse_array_literal(),
            Some(TokenType::Identifier(name)) => {
                let name_clone = name.clone();
                self.advance();

                if let Some(TokenType::LeftParen) = &self.current_token {
                    self.advance();

                    let mut arguments = Vec::new();

                    if let Some(TokenType::RightParen) = &self.current_token {
                        self.advance();
                        return Ok(Expression::FunctionCall(name_clone, arguments));
                    }

                    arguments.push(self.parse_expression()?);

                    while let Some(TokenType::Comma) = &self.current_token {
                        self.advance();
                        arguments.push(self.parse_expression()?);
                    }

                    if let Some(TokenType::RightParen) = &self.current_token {
                        self.advance();
                    } else {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ')' after function arguments", span)
                            .with_source(self.source_code.clone())
                            .with_hint("Close the argument list with ')'".to_string()));
                    }

                    Ok(Expression::FunctionCall(name_clone, arguments))
                } else if let Some(TokenType::LeftBrace) = &self.current_token {
                    self.advance();

                    let mut fields = Vec::new();

                    if let Some(TokenType::RightBrace) = &self.current_token {
                        self.advance();
                        return Ok(Expression::StructInstantiation(name_clone, fields));
                    }

                    let field_name = if let Some(TokenType::Identifier(field)) = &self.current_token
                    {
                        let field_clone = field.clone();
                        self.advance();
                        field_clone
                    } else {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error(
                            "Expected field name or '}' in struct literal",
                            span,
                        )
                        .with_source(self.source_code.clone())
                        .with_hint(
                            "Use: TypeName { field: value, ... } with each field as name: value."
                                .to_string(),
                        ));
                    };

                    if let Some(TokenType::Colon) = &self.current_token {
                        self.advance();
                    } else {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ':' after field name in struct literal", span)
                            .with_source(self.source_code.clone())
                            .with_hint(
                                "Use: field_name: expression for each struct field.".to_string(),
                            ));
                    }

                    let field_value = self.parse_expression()?;
                    fields.push((field_name, field_value));

                    while let Some(TokenType::Comma) = &self.current_token {
                        self.advance();

                        let field_name =
                            if let Some(TokenType::Identifier(field)) = &self.current_token {
                                let field_clone = field.clone();
                                self.advance();
                                field_clone
                            } else {
                                let span = Span::new(
                                    self.lexer.line,
                                    self.lexer.column,
                                    self.lexer.position,
                                    1,
                                );
                                return Err(parse_error(
                                    "Expected field name in struct literal",
                                    span,
                                )
                                .with_source(self.source_code.clone())
                                .with_hint(
                                    "After each comma, add field_name: value.".to_string(),
                                ));
                            };

                        if let Some(TokenType::Colon) = &self.current_token {
                            self.advance();
                        } else {
                            let span = Span::new(
                                self.lexer.line,
                                self.lexer.column,
                                self.lexer.position,
                                1,
                            );
                            return Err(parse_error(
                                "Expected ':' after field name in struct literal",
                                span,
                            )
                            .with_source(self.source_code.clone()));
                        }

                        let field_value = self.parse_expression()?;
                        fields.push((field_name, field_value));
                    }

                    if let Some(TokenType::RightBrace) = &self.current_token {
                        self.advance();
                    } else {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected '}' after struct fields", span)
                            .with_source(self.source_code.clone())
                            .with_hint("Close the struct literal with '}'".to_string()));
                    }

                    Ok(Expression::StructInstantiation(name_clone, fields))
                } else if let Some(TokenType::LeftBracket) = &self.current_token {
                    self.advance();
                    let index = self.parse_expression()?;

                    if let Some(TokenType::RightBracket) = &self.current_token {
                        self.advance();
                    } else {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ']' after array index", span)
                            .with_source(self.source_code.clone())
                            .with_hint("Close the index with ']'".to_string()));
                    }

                    let mut current_object = Expression::ArrayIndex(
                        Box::new(Expression::Identifier(name_clone)),
                        Box::new(index),
                    );

                    while let Some(TokenType::LeftBracket) = &self.current_token {
                        self.advance();
                        let index = self.parse_expression()?;

                        if let Some(TokenType::RightBracket) = &self.current_token {
                            self.advance();
                        } else {
                            let span = Span::new(
                                self.lexer.line,
                                self.lexer.column,
                                self.lexer.position,
                                1,
                            );
                            return Err(parse_error("Expected ']' after array index", span)
                                .with_source(self.source_code.clone())
                                .with_hint("Close the index with ']'".to_string()));
                        }

                        current_object =
                            Expression::ArrayIndex(Box::new(current_object), Box::new(index));
                    }

                    if let Some(TokenType::Dot) = &self.current_token {
                        self.parse_method_call_chain(current_object)
                    } else {
                        Ok(current_object)
                    }
                } else if let Some(TokenType::Colon) = &self.current_token {
                    if let Some(TokenType::Colon) = self.lexer.peek_token() {
                        self.advance();
                        self.advance();

                        let variant_name =
                            if let Some(TokenType::Identifier(variant)) = &self.current_token {
                                let variant_clone = variant.clone();
                                self.advance();
                                variant_clone
                            } else {
                                let span = Span::new(
                                    self.lexer.line,
                                    self.lexer.column,
                                    self.lexer.position,
                                    1,
                                );
                                return Err(parse_error("Expected variant name after '::'", span)
                                    .with_source(self.source_code.clone())
                                    .with_hint(
                                        "Use: EnumName::VariantName for enum values.".to_string(),
                                    ));
                            };

                        Ok(Expression::EnumVariant(name_clone, variant_name))
                    } else {
                        Ok(Expression::Identifier(name_clone))
                    }
                } else if let Some(TokenType::Dot) = &self.current_token {
                    let object = Expression::Identifier(name_clone);
                    self.parse_method_call_chain(object)
                } else {
                    Ok(Expression::Identifier(name_clone))
                }
            }
            Some(tok) => {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                Err(parse_error(format!("Unexpected token in expression: {:?}", tok), span)
                    .with_source(self.source_code.clone())
                    .with_hint(
                        "Expected a literal, identifier, '(', or '[' to start an expression."
                            .to_string(),
                    ))
            }
            None => {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                Err(parse_error("Unexpected end of input in expression", span)
                    .with_source(self.source_code.clone()))
            }
        }
    }

    fn parse_method_call_chain(
        &mut self,
        object: Expression,
    ) -> Result<Expression, RavenError> {
        let mut current_object = object;

        while let Some(TokenType::Dot) = &self.current_token {
            self.advance();

            let name = if let Some(TokenType::Identifier(n)) = &self.current_token {
                let name_clone = n.clone();
                self.advance();
                name_clone
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected method or field name after '.'", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Use: value.method(...) or value.field".to_string()));
            };

            if let Some(TokenType::LeftParen) = &self.current_token {
                self.advance();

                let mut arguments = Vec::new();

                if let Some(TokenType::RightParen) = &self.current_token {
                    self.advance();
                    current_object =
                        Expression::MethodCall(Box::new(current_object), name, arguments);
                    continue;
                }

                arguments.push(self.parse_expression()?);

                while let Some(TokenType::Comma) = &self.current_token {
                    self.advance();
                    arguments.push(self.parse_expression()?);
                }

                if let Some(TokenType::RightParen) = &self.current_token {
                    self.advance();
                } else {
                    let span =
                        Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(parse_error("Expected ')' after method arguments", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Close the argument list with ')'".to_string()));
                }

                current_object = Expression::MethodCall(Box::new(current_object), name, arguments);
            } else {
                current_object = Expression::FieldAccess(Box::new(current_object), name);
            }

            while let Some(TokenType::LeftBracket) = &self.current_token {
                self.advance();
                let index = self.parse_expression()?;

                if let Some(TokenType::RightBracket) = &self.current_token {
                    self.advance();
                } else {
                    let span =
                        Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(parse_error("Expected ']' after array index", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Close the index with ']'".to_string()));
                }

                current_object = Expression::ArrayIndex(Box::new(current_object), Box::new(index));
            }
        }

        Ok(current_object)
    }

    fn parse_array_literal(&mut self) -> Result<Expression, RavenError> {
        self.advance();

        let mut elements = Vec::new();

        if let Some(TokenType::RightBracket) = &self.current_token {
            self.advance();
            return Ok(Expression::ArrayLiteral(elements));
        }

        elements.push(self.parse_expression()?);

        while let Some(TokenType::Comma) = &self.current_token {
            self.advance();
            elements.push(self.parse_expression()?);
        }

        if let Some(TokenType::RightBracket) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected ']' after array elements", span)
                .with_source(self.source_code.clone())
                .with_hint("Close the array literal with ']'".to_string()));
        }

        Ok(Expression::ArrayLiteral(elements))
    }

    fn parse_print_statement(&mut self) -> Result<ASTNode, RavenError> {
        let print_start_line = self.lexer.line;

        self.advance();

        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance();

            let mut arguments = Vec::new();

            if let Some(TokenType::RightParen) = &self.current_token {
                self.advance();
            } else {
                arguments.push(self.parse_expression()?);

                while let Some(TokenType::Comma) = &self.current_token {
                    self.advance();
                    arguments.push(self.parse_expression()?);
                }

                if let Some(TokenType::RightParen) = &self.current_token {
                    self.advance();
                } else {
                    let span =
                        Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(parse_error("Expected ')' after print arguments", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Close the parenthesis with ')'".to_string()));
                }
            }

            if let Some(TokenType::Semicolon) = &self.current_token {
                self.advance();

                return Ok(ASTNode::FunctionCall("print".to_string(), arguments));
            } else {
                let lines: Vec<&str> = self.source_code.lines().collect();
                let error_column = if print_start_line < lines.len() {
                    lines[print_start_line].len()
                } else {
                    self.lexer.column
                };
                let span = Span::new(print_start_line, error_column, self.lexer.position, 1);
                return Err(parse_error("Expected ';' after print statement", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add ';' at the end".to_string()));
            }
        }

        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
        Err(parse_error("Expected '(' after 'print'", span)
            .with_source(self.source_code.clone())
            .with_hint("Use: print(expression);".to_string()))
    }

    fn parse_function_declaration(&mut self) -> Result<ASTNode, RavenError> {
        self.advance();

        let func_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone = name.clone();
            self.advance();
            name_clone
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected function name after 'fun'", span)
                .with_source(self.source_code.clone())
                .with_hint("Provide a function name".to_string()));
        };

        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '(' after function name", span)
                .with_source(self.source_code.clone())
                .with_hint("Add '(' to start parameter list".to_string()));
        }

        let mut parameters = Vec::new();
        while let Some(token) = &self.current_token {
            if let TokenType::RightParen = token {
                break;
            }

            let param_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                name_clone
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected parameter name", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Provide a parameter name".to_string()));
            };

            if let Some(TokenType::Colon) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ':' after parameter name", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add ':' followed by the parameter type".to_string()));
            }

            let param_type = self.parse_type_string()?;

            parameters.push(crate::ast::Parameter {
                name: param_name,
                param_type,
            });

            if let Some(TokenType::Comma) = &self.current_token {
                self.advance();
            }
        }

        if let Some(TokenType::RightParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected ')' after parameters", span)
                .with_source(self.source_code.clone())
                .with_hint("Close the parameter list with ')'".to_string()));
        }

        let return_type = if let Some(TokenType::Arrow) = &self.current_token {
            self.advance();
            self.parse_type_string()?
        } else {
            "void".to_string()
        };

        if let Some(TokenType::LeftBrace) = &self.current_token {
            self.advance();
            let body = self.parse_block()?;

            if let Some(TokenType::RightBrace) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected '}' to close function body", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add '}' to close the function body".to_string()));
            }

            Ok(ASTNode::FunctionDecl(
                func_name,
                return_type,
                parameters,
                Box::new(body),
            ))
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            Err(parse_error("Expected '{' to start function body", span)
                .with_source(self.source_code.clone())
                .with_hint("Add '{' to start the function body".to_string()))
        }
    }

    fn parse_struct_declaration(&mut self) -> Result<ASTNode, RavenError> {
        self.advance();

        let struct_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone = name.clone();
            self.advance();
            name_clone
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected struct name after 'struct'", span)
                .with_source(self.source_code.clone())
                .with_hint("Provide a struct name".to_string()));
        };

        if let Some(TokenType::LeftBrace) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '{' after struct name", span)
                .with_source(self.source_code.clone())
                .with_hint("Add '{' to start struct body".to_string()));
        }

        let mut members = Vec::new();
        while let Some(token) = &self.current_token {
            if let TokenType::RightBrace = token {
                break;
            }

            if let TokenType::Comment(text) = token {
                let text = text.clone();
                self.advance();
                members.push(StructMember::Comment(text));
                continue;
            }

            let field_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                name_clone
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected field name", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Provide a field name".to_string()));
            };

            if let Some(TokenType::Colon) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ':' after field name", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add ':' followed by the field type".to_string()));
            }

            let field_type = self.parse_type_string()?;

            members.push(StructMember::Field(crate::ast::StructField {
                name: field_name,
                field_type,
            }));

            if let Some(TokenType::Comma) = &self.current_token {
                self.advance();
            }
        }

        if let Some(TokenType::RightBrace) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '}' to close struct body", span)
                .with_source(self.source_code.clone())
                .with_hint("Add '}' to close the struct body".to_string()));
        }

        Ok(ASTNode::StructDecl(struct_name, members))
    }

    fn parse_impl_block(&mut self) -> Result<ASTNode, RavenError> {
        use crate::ast::Parameter;

        self.advance();

        let struct_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone = name.clone();
            self.advance();
            name_clone
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected struct name after 'impl'", span)
                .with_source(self.source_code.clone())
                .with_hint("Use: impl StructName { fun method(self, ...) { ... } }".to_string()));
        };

        if !matches!(&self.current_token, Some(TokenType::LeftBrace)) {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '{' after struct name", span)
                .with_source(self.source_code.clone())
                .with_hint("Add '{' to start impl block".to_string()));
        }
        self.advance();

        let mut methods = Vec::new();
        while !matches!(&self.current_token, Some(TokenType::RightBrace)) {
            if let Some(TokenType::Comment(text)) = &self.current_token {
                let text = text.clone();
                self.advance();
                methods.push(ImplMember::Comment(text));
                continue;
            }
            if !matches!(&self.current_token, Some(TokenType::Fun)) {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected 'fun' for method in impl block", span)
                    .with_source(self.source_code.clone())
                    .with_hint(
                        "Methods must start with 'fun method_name(self, ...)'".to_string(),
                    ));
            }
            self.advance();

            let method_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                name_clone
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected method name", span).with_source(self.source_code.clone())
                );
            };

            if !matches!(&self.current_token, Some(TokenType::LeftParen)) {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected '(' after method name", span)
                    .with_source(self.source_code.clone()));
            }
            self.advance();

            let param_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                name_clone
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected 'self' as first parameter", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Methods must have 'self' as first parameter".to_string()));
            };
            if param_name != "self" {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error(
                    format!("Expected 'self' as first parameter, got '{}'", param_name),
                    span,
                )
                .with_source(self.source_code.clone()));
            }
            if matches!(&self.current_token, Some(TokenType::Colon)) {
                self.advance();
                let _ = self.parse_type_string()?;
            }
            let mut parameters = vec![Parameter {
                name: "self".to_string(),
                param_type: struct_name.clone(),
            }];

            if matches!(&self.current_token, Some(TokenType::Comma)) {
                self.advance();
                while !matches!(&self.current_token, Some(TokenType::RightParen)) {
                    let pname = if let Some(TokenType::Identifier(n)) = &self.current_token {
                        let c = n.clone();
                        self.advance();
                        c
                    } else {
                        break;
                    };
                    if !matches!(&self.current_token, Some(TokenType::Colon)) {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ':' after parameter name", span)
                            .with_source(self.source_code.clone()));
                    }
                    self.advance();
                    let ptype = self.parse_type_string()?;
                    parameters.push(Parameter {
                        name: pname,
                        param_type: ptype,
                    });
                    if !matches!(&self.current_token, Some(TokenType::Comma)) {
                        break;
                    }
                    self.advance();
                }
            }

            if !matches!(&self.current_token, Some(TokenType::RightParen)) {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ')' after parameters", span)
                    .with_source(self.source_code.clone()));
            }
            self.advance();

            let return_type = self.parse_return_type()?;

            if !matches!(&self.current_token, Some(TokenType::LeftBrace)) {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected '{' for method body", span)
                    .with_source(self.source_code.clone()));
            }
            self.advance();
            let body = self.parse_block()?;
            if !matches!(&self.current_token, Some(TokenType::RightBrace)) {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected '}' to close method body", span)
                    .with_source(self.source_code.clone()));
            }
            self.advance();

            methods.push(ImplMember::Method(
                method_name,
                return_type,
                parameters,
                Box::new(body),
            ));
        }

        if !matches!(&self.current_token, Some(TokenType::RightBrace)) {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '}' to close impl block", span)
                .with_source(self.source_code.clone()));
        }
        self.advance();

        Ok(ASTNode::ImplBlock(struct_name, methods))
    }

    fn parse_return_type(&mut self) -> Result<String, RavenError> {
        if matches!(&self.current_token, Some(TokenType::Arrow)) {
            self.advance();
            self.parse_type_string()
        } else {
            Ok("void".to_string())
        }
    }

    fn parse_while_loop(&mut self) -> Result<ASTNode, RavenError> {
        self.advance();

        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '(' after 'while'", span)
                .with_source(self.source_code.clone())
                .with_hint("Use: while (condition) { ... }".to_string()));
        }

        let condition = self.parse_expression()?;

        if let Some(TokenType::RightParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected ')' after while condition", span)
                .with_source(self.source_code.clone())
                .with_hint("Close the condition with ')'".to_string()));
        }

        if let Some(TokenType::LeftBrace) = &self.current_token {
            self.advance();
            let body = self.parse_block()?;

            if let Some(TokenType::RightBrace) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected '}' to close while body", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add '}' to close the loop body".to_string()));
            }

            Ok(ASTNode::WhileLoop(Box::new(condition), Box::new(body)))
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            Err(parse_error("Expected '{' to start while body", span)
                .with_source(self.source_code.clone())
                .with_hint("Add '{' after the condition".to_string()))
        }
    }

    fn parse_for_loop(&mut self) -> Result<ASTNode, RavenError> {
        self.advance();

        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '(' after 'for'", span)
                .with_source(self.source_code.clone())
                .with_hint("Use: for (let i: int = 0; i < 10; i = i + 1) { ... }".to_string()));
        }

        let init = if let Some(TokenType::Let) = &self.current_token {
            self.parse_variable_declaration()?
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error(
                "Expected variable declaration in for loop initialization",
                span,
            )
            .with_source(self.source_code.clone())
            .with_hint("Use 'let' to declare the loop variable".to_string()));
        };

        let condition = self.parse_expression()?;

        if let Some(TokenType::Semicolon) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected ';' after for loop condition", span)
                .with_source(self.source_code.clone())
                .with_hint(
                    "For loops use: for (init; condition; increment) { ... }.\n\
                     This means you need the ';' after the condition and then an increment clause before ')'.\n\
                     Example: for (let i: int = 0; i < 10; i = i + 1) { ... }"
                        .to_string(),
                ));
        }

        let increment = if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone = name.clone();
            self.advance();

            if let Some(TokenType::Assign) = &self.current_token {
                self.advance();
                let expr = self.parse_expression()?;
                ASTNode::Assignment(Box::new(Expression::Identifier(name_clone)), Box::new(expr))
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected '=' in for loop increment", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Use: i = i + 1".to_string()));
            }
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected assignment in for loop increment", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Provide an assignment like: i = i + 1".to_string()),
            );
        };

        if let Some(TokenType::RightParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected ')' after for loop header", span)
                .with_source(self.source_code.clone())
                .with_hint(
                    "Close the for loop header with ')', after the increment expression.\n\
                     For loops use: for (init; condition; increment) { ... }"
                        .to_string(),
                ));
        }

        if let Some(TokenType::LeftBrace) = &self.current_token {
            self.advance();
            let body = self.parse_block()?;

            if let Some(TokenType::RightBrace) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected '}' to close for body", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add '}' to close the loop body".to_string()));
            }

            Ok(ASTNode::ForLoop(
                Box::new(init),
                Box::new(condition),
                Box::new(increment),
                Box::new(body),
            ))
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            Err(parse_error("Expected '{' to start for body", span)
                .with_source(self.source_code.clone())
                .with_hint(
                    "Add '{' to start the loop body.\n\
                     Full syntax: for (init; condition; increment) { ... }"
                        .to_string(),
                ))
        }
    }

    fn parse_return_statement(&mut self) -> Result<ASTNode, RavenError> {
        self.advance();

        let expr_start_line = self.lexer.line;
        let expr = self.parse_expression()?;

        if let Some(TokenType::Semicolon) = &self.current_token {
            self.advance();
            Ok(ASTNode::Return(Box::new(expr)))
        } else {
            let error_line = if self.lexer.line > expr_start_line {
                expr_start_line
            } else {
                self.lexer.line
            };

            let lines: Vec<&str> = self.source_code.lines().collect();
            let error_column = if error_line < lines.len() {
                lines[error_line].len()
            } else {
                self.lexer.column
            };

            let span = Span::new(error_line, error_column, self.lexer.position, 1);
            Err(parse_error("Expected ';' after return statement", span)
                .with_source(self.source_code.clone())
                .with_hint("Add ';' at the end of the statement".to_string()))
        }
    }

    fn parse_import_statement(&mut self) -> Result<ASTNode, RavenError> {
        self.advance();

        if let Some(TokenType::LeftBrace) = &self.current_token {
            self.advance();

            let mut items = Vec::new();

            if let Some(TokenType::Identifier(item)) = &self.current_token {
                items.push(item.clone());
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected identifier in import list", span)
                    .with_source(self.source_code.clone()));
            }

            while let Some(TokenType::Comma) = &self.current_token {
                self.advance();

                if let Some(TokenType::Identifier(item)) = &self.current_token {
                    items.push(item.clone());
                    self.advance();
                } else {
                    let span =
                        Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(parse_error(
                        "Expected identifier after comma in import list",
                        span,
                    )
                    .with_source(self.source_code.clone()));
                }
            }

            if let Some(TokenType::RightBrace) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected '}' after import list", span)
                    .with_source(self.source_code.clone()));
            }

            if let Some(TokenType::From) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected 'from' after import list", span)
                    .with_source(self.source_code.clone()));
            }

            let module_name = if let Some(TokenType::StringLiteral(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                name_clone
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected module name (string) after 'from'", span)
                        .with_source(self.source_code.clone()),
                );
            };

            if let Some(TokenType::Semicolon) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ';' after import statement", span)
                    .with_source(self.source_code.clone()));
            }

            Ok(ASTNode::ImportSelective(module_name, items))
        } else {
            let module_name = if let Some(TokenType::StringLiteral(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                name_clone
            } else if let Some(TokenType::Identifier(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();

                if let Some(TokenType::From) = &self.current_token {
                    self.advance();

                    let module_path = if let Some(TokenType::StringLiteral(path)) =
                        &self.current_token
                    {
                        let path_clone = path.clone();
                        self.advance();
                        path_clone
                    } else {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error(
                            "Expected module path (string) after 'from'",
                            span,
                        )
                        .with_source(self.source_code.clone()));
                    };

                    if let Some(TokenType::Semicolon) = &self.current_token {
                        self.advance();
                    } else {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ';' after import statement", span)
                            .with_source(self.source_code.clone()));
                    }

                    return Ok(ASTNode::Import(module_path, Some(name_clone)));
                } else {
                    if let Some(TokenType::Semicolon) = &self.current_token {
                        self.advance();
                    } else {
                        let span =
                            Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ';' after import statement", span)
                            .with_source(self.source_code.clone()));
                    }

                    return Ok(ASTNode::Import(name_clone, None));
                }
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected module name or identifier after 'import'", span)
                        .with_source(self.source_code.clone()),
                );
            };

            if let Some(TokenType::Semicolon) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ';' after import statement", span)
                    .with_source(self.source_code.clone()));
            }

            Ok(ASTNode::Import(module_name, None))
        }
    }

    fn parse_export_statement(&mut self) -> Result<ASTNode, RavenError> {
        self.advance();

        let stmt = match &self.current_token {
            Some(TokenType::Let) => self.parse_variable_declaration()?,
            Some(TokenType::Fun) => self.parse_function_declaration()?,
            _ => {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected 'let' or 'fun' after 'export'", span)
                    .with_source(self.source_code.clone()));
            }
        };

        Ok(ASTNode::Export(Box::new(stmt)))
    }

    fn parse_enum_declaration(&mut self) -> Result<ASTNode, RavenError> {
        self.advance();

        let enum_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone = name.clone();
            self.advance();
            name_clone
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected enum name after 'enum'", span)
                .with_source(self.source_code.clone())
                .with_hint("Provide an enum name".to_string()));
        };

        if let Some(TokenType::LeftBrace) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '{' after enum name", span)
                .with_source(self.source_code.clone())
                .with_hint("Add '{' to start enum body".to_string()));
        }

        let mut members = Vec::new();
        while let Some(token) = &self.current_token {
            if let TokenType::RightBrace = token {
                break;
            }

            if let TokenType::Comment(text) = token {
                let text = text.clone();
                self.advance();
                members.push(EnumMember::Comment(text));
                continue;
            }

            let variant_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                name_clone
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected variant name", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Provide a variant name".to_string()));
            };

            members.push(EnumMember::Variant(variant_name));

            if let Some(TokenType::Comma) = &self.current_token {
                self.advance();
            } else if let Some(TokenType::RightBrace) = &self.current_token {
                break;
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ',' or '}' after variant", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add ',' to separate variants or '}' to end enum".to_string()));
            }
        }

        if let Some(TokenType::RightBrace) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '}' to close enum", span)
                .with_source(self.source_code.clone())
                .with_hint("Add '}' to close the enum".to_string()));
        }

        Ok(ASTNode::EnumDecl(enum_name, members))
    }
}
