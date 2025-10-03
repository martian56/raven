use crate::lexer::{Lexer, TokenType};
use crate::ast::{ASTNode, Expression, Operator, Type};
use crate::error::{RavenError, parse_error};
use crate::span::Span;

pub struct Parser {
    lexer: Lexer,
    current_token: Option<TokenType>,
    source_code: String,  // Keep source for error reporting
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
                TokenType::Identifier(_) => {
                    // Check if this is a function call or method call statement
                    // We need to peek ahead to see what comes after the identifier
                    let next_token = self.lexer.peek_token();
                    
                    if let Some(TokenType::LeftParen) = next_token {
                        // It's a function call statement
                        let expr = self.parse_expression();
                        // Expect semicolon
                        if let Some(TokenType::Semicolon) = &self.current_token {
                            self.advance(); // Skip ';'
                        } else {
                            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                            return Err(
                                parse_error("Expected ';' after function call", span)
                                    .with_source(self.source_code.clone())
                                    .with_hint("Add ';' at the end".to_string())
                            );
                        }
                        // Extract function call from expression
                        if let Expression::FunctionCall(name, args) = expr {
                            ASTNode::FunctionCall(name, args)
                        } else {
                            return Err(parse_error("Expected function call", Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1))
                                .with_source(self.source_code.clone()));
                        }
                    } else if let Some(TokenType::Dot) = next_token {
                        // It's a method call statement
                        let expr = self.parse_expression();
                        // Expect semicolon
                        if let Some(TokenType::Semicolon) = &self.current_token {
                            self.advance(); // Skip ';'
                        } else {
                            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                            return Err(
                                parse_error("Expected ';' after method call", span)
                                    .with_source(self.source_code.clone())
                                    .with_hint("Add ';' at the end".to_string())
                            );
                        }
                        // Extract method call from expression
                        if let Expression::MethodCall(object, method, args) = expr {
                            ASTNode::MethodCall(object, method, args)
                        } else {
                            return Err(parse_error("Expected method call", Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1))
                                .with_source(self.source_code.clone()));
                        }
                    } else {
                        // It's an assignment
                        self.parse_assignment()?
                    }
                },
                TokenType::If => self.parse_if_statement()?,
                TokenType::While => self.parse_while_loop()?,
                TokenType::For => self.parse_for_loop()?,
                TokenType::Fun => self.parse_function_declaration()?,
                TokenType::Return => self.parse_return_statement()?,
                TokenType::Print => self.parse_print_statement()?,
                TokenType::Import => self.parse_import_statement()?,
                TokenType::Export => self.parse_export_statement()?,
                TokenType::EOF => break,
                _ => return Err(format!("Unexpected token: {:?}", token).into()),
            };
    
            statements.push(stmt);
        }
    
        Ok(ASTNode::Block(statements))
    }
    
    
    fn parse_variable_declaration(&mut self) -> Result<ASTNode, RavenError> {
        self.advance(); // Skip 'let'
    
        // Step 1: Expect identifier
        let var_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone = name.clone();
            self.advance(); // Skip identifier
            name_clone
        } else {
            return Err("Expected identifier after 'let'.".to_string().into());
        };
    
        // Step 2: Expect colon
        if let Some(TokenType::Colon) = &self.current_token {
            self.advance(); // Skip ':'
        } else {
            return Err("Expected ':' after variable name.".to_string().into());
        }
    
        // Step 3: Expect a type (IntType, FloatType, etc.)
        let var_type = match &self.current_token {
            Some(TokenType::IntType) => {
                self.advance();
                
                // Check if this is an array type: int[]
                if let Some(TokenType::LeftBracket) = &self.current_token {
                    self.advance(); // Skip '['
                    
                    // Expect ']'
                    if let Some(TokenType::RightBracket) = &self.current_token {
                        self.advance(); // Skip ']'
                        "int[]".to_string()
                    } else {
                        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ']' after array type", span)
                            .with_source(self.source_code.clone()));
                    }
                } else {
                    "int".to_string()
                }
            }
            Some(TokenType::FloatType) => {
                self.advance();
                
                // Check if this is an array type: float[]
                if let Some(TokenType::LeftBracket) = &self.current_token {
                    self.advance(); // Skip '['
                    
                    // Expect ']'
                    if let Some(TokenType::RightBracket) = &self.current_token {
                        self.advance(); // Skip ']'
                        "float[]".to_string()
                    } else {
                        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ']' after array type", span)
                            .with_source(self.source_code.clone()));
                    }
                } else {
                    "float".to_string()
                }
            }
            Some(TokenType::BoolType) => {
                self.advance();
                
                // Check if this is an array type: bool[]
                if let Some(TokenType::LeftBracket) = &self.current_token {
                    self.advance(); // Skip '['
                    
                    // Expect ']'
                    if let Some(TokenType::RightBracket) = &self.current_token {
                        self.advance(); // Skip ']'
                        "bool[]".to_string()
                    } else {
                        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ']' after array type", span)
                            .with_source(self.source_code.clone()));
                    }
                } else {
                    "bool".to_string()
                }
            }
            Some(TokenType::StringType) => {
                self.advance();
                
                // Check if this is an array type: String[]
                if let Some(TokenType::LeftBracket) = &self.current_token {
                    self.advance(); // Skip '['
                    
                    // Expect ']'
                    if let Some(TokenType::RightBracket) = &self.current_token {
                        self.advance(); // Skip ']'
                        "String[]".to_string()
                    } else {
                        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ']' after array type", span)
                            .with_source(self.source_code.clone()));
                    }
                } else {
                    "string".to_string()
                }
            }
            Some(TokenType::VoidType) => {
                self.advance();
                "void".to_string()
            }
            other => {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error(&format!("Expected type after ':', got {:?}", other), span)
                    .with_source(self.source_code.clone()));
            }
        };
    
        // Step 4: Expect '='
        if let Some(TokenType::Assign) = &self.current_token {
            self.advance(); // Skip '='
        } else {
            return Err("Expected '=' after variable type.".to_string().into());
        }
    
        // Step 5: Parse the expression (track position before for error reporting)
        let expr_start_line = self.lexer.line;
        let expr = self.parse_expression();
    
        // Step 6: Expect semicolon
        if let Some(TokenType::Semicolon) = &self.current_token {
            self.advance(); // Skip ';'
        } else {
            // If we're on a different line, the semicolon should be at end of previous line
            let error_line = if self.lexer.line > expr_start_line {
                // We're on a new line, so point to end of previous line
                expr_start_line
            } else {
                // Same line, point to current position
                self.lexer.line
            };
            
            // For better UX, find the end of the previous line
            let lines: Vec<&str> = self.source_code.lines().collect();
            let error_column = if error_line < lines.len() {
                lines[error_line].len()
            } else {
                self.lexer.column
            };
            
            let span = Span::new(error_line, error_column, self.lexer.position, 1);
            return Err(
                parse_error("Expected ';' after variable declaration", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add ';' at the end of the statement".to_string())
            );
        }
    
        Ok(ASTNode::VariableDeclTyped(var_name, var_type, Box::new(expr)))
    }
    

    fn parse_function_call_statement(&mut self) -> Result<ASTNode, RavenError> {
        // Parse function name
        let name = if let Some(TokenType::Identifier(n)) = &self.current_token {
            n.clone()
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected function name", span)
                .with_source(self.source_code.clone()));
        };
        self.advance(); // Consume the identifier
        
        // Expect '('
        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance(); // Skip '('
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected '(' after function name", span)
                .with_source(self.source_code.clone()));
        }
        
        // Parse arguments
        let mut arguments = Vec::new();
        
        // Check for empty argument list
        if let Some(TokenType::RightParen) = &self.current_token {
            self.advance(); // Skip ')'
        } else {
            // Parse first argument
            arguments.push(self.parse_expression());
            
            // Parse remaining arguments
            while let Some(TokenType::Comma) = &self.current_token {
                self.advance(); // Skip ','
                arguments.push(self.parse_expression());
            }
            
            // Expect ')'
            if let Some(TokenType::RightParen) = &self.current_token {
                self.advance(); // Skip ')'
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ')' after function arguments", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Close the function call with ')'".to_string()));
            }
        }
        
        // Expect semicolon
        if let Some(TokenType::Semicolon) = &self.current_token {
            self.advance(); // Skip ';'
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(parse_error("Expected ';' after function call", span)
                .with_source(self.source_code.clone())
                .with_hint("Add ';' at the end".to_string()));
        }
        
        Ok(ASTNode::FunctionCall(name, arguments))
    }

    fn parse_assignment(&mut self) -> Result<ASTNode, RavenError> {
        if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone: String = name.clone();
            let name_line = self.lexer.line;
            self.advance(); // Skip identifier

            if let Some(TokenType::Assign) = &self.current_token {
                self.advance(); // Skip '='
                let expr_start_line = self.lexer.line;
                let expr: Expression = self.parse_expression();

                if let Some(TokenType::Semicolon) = &self.current_token {
                    self.advance(); // Skip ';'
                    return Ok(ASTNode::Assignment(name_clone, Box::new(expr)));
                } else {
                    // Same logic as variable declaration - point to correct line
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
                    return Err(
                        parse_error("Expected ';' after assignment", span)
                            .with_source(self.source_code.clone())
                            .with_hint("Add ';' at the end of the statement".to_string())
                    );
                }
            }
        }

        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
        Err(parse_error("Invalid assignment", span)
            .with_source(self.source_code.clone()))
    }

    fn parse_if_statement(&mut self) -> Result<ASTNode, RavenError> {
        self.parse_if_like_statement(true)
    }
    
    fn parse_if_like_statement(&mut self, is_if: bool) -> Result<ASTNode, RavenError> {
        self.advance(); // Skip 'If' or 'ElseIf'
    
        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance(); // Skip '('
            let condition: Expression = self.parse_expression();
    
            if let Some(TokenType::RightParen) = &self.current_token {
                self.advance(); // Skip ')'
    
                if let Some(TokenType::LeftBrace) = &self.current_token {
                    self.advance(); // Skip '{'
                    let body: ASTNode = self.parse_block()?;
                    self.advance(); // Skip '}'
    
                    let mut else_if = None;
                    let mut else_block = None;
    
                    while let Some(token) = &self.current_token {
                        match token {
                            TokenType::ElseIf => {
                                else_if = Some(self.parse_if_like_statement(false)?);
                                break;
                            }
                            TokenType::Else => {
                                self.advance(); // Skip 'Else'
                                if let Some(TokenType::LeftBrace) = &self.current_token {
                                    self.advance(); // Skip '{'
                                    else_block = Some(self.parse_block()?);
                                    self.advance(); // Skip '}'
                                    break;
                                } else {
                                    let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                                    return Err(
                                        parse_error("Expected '{' after 'else'", span)
                                            .with_source(self.source_code.clone())
                                            .with_hint("Add '{' to start the else block".to_string())
                                    );
                                }
                            }
                            _ => break,
                        }
                    }
    
                    return Ok(ASTNode::IfStatement(
                        Box::new(condition),
                        Box::new(body),
                        else_if.map(Box::new),
                        else_block.map(Box::new),
                    ));
                } else {
                    let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(
                        parse_error("Expected '{' to start if block", span)
                            .with_source(self.source_code.clone())
                            .with_hint("Add '{' after the condition".to_string())
                    );
                }
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected ')' after condition", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Close the condition with ')'".to_string())
                );
            }
        }
    
        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
        let keyword = if is_if { "if" } else { "elseif" };
        Err(parse_error(format!("Expected '(' after '{}'", keyword), span)
            .with_source(self.source_code.clone())
            .with_hint(format!("Use: {} (condition) {{ ... }}", keyword)))
    }
    


    fn parse_block(&mut self) -> Result<ASTNode, RavenError> {
        let mut statements = Vec::new();
    
        while let Some(token) = &self.current_token {
            if let TokenType::RightBrace = token {
                break; // Stop before consuming the closing brace
            }
    
            let stmt = match token {
                TokenType::Let => self.parse_variable_declaration()?,
                TokenType::Identifier(_) => self.parse_assignment()?,
                TokenType::If => self.parse_if_statement()?,
                TokenType::While => self.parse_while_loop()?,
                TokenType::For => self.parse_for_loop()?,
                TokenType::Return => self.parse_return_statement()?,
                TokenType::Print => self.parse_print_statement()?,
                _ => return Err(format!("Unexpected token in block: {:?}", token).into()),
            };
    
            statements.push(stmt);
        }
    
        Ok(ASTNode::Block(statements))
    }
    

    fn parse_expression(&mut self) -> Expression {
        self.parse_expression_with_precedence(0)
    }

    // Precedence climbing algorithm for correct operator precedence
    fn parse_expression_with_precedence(&mut self, min_precedence: u8) -> Expression {
        let mut left = self.parse_term();

        while let Some(op) = self.match_operator() {
            let precedence = self.operator_precedence(&op);
            
            // Only continue if this operator has higher or equal precedence
            if precedence < min_precedence {
                break;
            }

            self.advance(); // Skip operator

            // Parse the right side with higher precedence for left-associativity
            let right = self.parse_expression_with_precedence(precedence + 1);
            
            left = Expression::BinaryOp(Box::new(left), op, Box::new(right));
        }

        left
    }

    // Operator precedence levels (higher number = higher precedence)
    fn operator_precedence(&self, op: &Operator) -> u8 {
        match op {
            Operator::Or => 1,                                    // Lowest
            Operator::And => 2,
            Operator::Equal | Operator::NotEqual => 3,
            Operator::LessThan | Operator::GreaterThan 
            | Operator::LessEqual | Operator::GreaterEqual => 4,
            Operator::Add | Operator::Subtract => 5,
            Operator::Multiply | Operator::Divide => 6,           // Highest
        }
    }

    fn match_operator(&self) -> Option<Operator> {
        match self.current_token {
            Some(TokenType::Plus) => Some(Operator::Add),
            Some(TokenType::Minus) => Some(Operator::Subtract),
            Some(TokenType::Star) => Some(Operator::Multiply),
            Some(TokenType::Slash) => Some(Operator::Divide),
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
    

    fn parse_term(&mut self) -> Expression {
        match &self.current_token {
            Some(TokenType::IntLiteral(value)) => {
                let val = *value;
                self.advance();
                Expression::Integer(val)
            }
            Some(TokenType::FloatLiteral(value)) => {
                let val = *value;
                self.advance();
                Expression::Float(val)
            }
            Some(TokenType::BoolLiteral(value)) => {
                let val = *value;
                self.advance();
                Expression::Boolean(val)
            }
            Some(TokenType::StringLiteral(s)) => {
                let s_clone = s.clone();
                self.advance();
                Expression::StringLiteral(s_clone)
            }
            Some(TokenType::LeftBracket) => {
                // Array literal: [1, 2, 3]
                self.parse_array_literal()
            }
            Some(TokenType::Identifier(name)) => {
                let name_clone = name.clone();
                self.advance();
                
                // Check if this is a function call
                if let Some(TokenType::LeftParen) = &self.current_token {
                    self.advance(); // Skip '('
                    
                    // Parse arguments
                    let mut arguments = Vec::new();
                    
                    // Check for empty argument list
                    if let Some(TokenType::RightParen) = &self.current_token {
                        self.advance(); // Skip ')'
                        return Expression::FunctionCall(name_clone, arguments);
                    }
                    
                    // Parse first argument
                    arguments.push(self.parse_expression());
                    
                    // Parse remaining arguments
                    while let Some(TokenType::Comma) = &self.current_token {
                        self.advance(); // Skip ','
                        arguments.push(self.parse_expression());
                    }
                    
                    // Expect ')'
                    if let Some(TokenType::RightParen) = &self.current_token {
                        self.advance(); // Skip ')'
                    } else {
                        panic!("Expected ')' after function arguments");
                    }
                    
                    Expression::FunctionCall(name_clone, arguments)
                } else {
                    // Check if this is array indexing: array[index]
                    if let Some(TokenType::LeftBracket) = &self.current_token {
                        self.advance(); // Skip '['
                        let index = self.parse_expression();
                        
                        // Expect ']'
                        if let Some(TokenType::RightBracket) = &self.current_token {
                            self.advance(); // Skip ']'
                        } else {
                            panic!("Expected ']' after array index");
                        }
                        
                        Expression::ArrayIndex(Box::new(Expression::Identifier(name_clone)), Box::new(index))
                    } else {
                        // Check if this is a method call: object.method(args)
                        if let Some(TokenType::Dot) = &self.current_token {
                            // Parse the initial identifier as the object
                            let object = Expression::Identifier(name_clone);
                            // Use the chained method call parser
                            self.parse_method_call_chain(object)
                        } else {
                            // Just a variable reference
                            Expression::Identifier(name_clone)
                        }
                    }
                }
            }
            _ => panic!("Unexpected token in expression: {:?}", self.current_token),
        }
    }

    /// Parse chained method calls: object.method1().method2().method3()
    fn parse_method_call_chain(&mut self, object: Expression) -> Expression {
        let mut current_object = object;
        
        while let Some(TokenType::Dot) = &self.current_token {
            self.advance(); // Skip '.'
            
            // Expect method name
            let method_name = if let Some(TokenType::Identifier(method)) = &self.current_token {
                let method_clone = method.clone();
                self.advance();
                method_clone
            } else {
                panic!("Expected method name after '.'");
            };
            
            // Expect '(' for method call
            if let Some(TokenType::LeftParen) = &self.current_token {
                self.advance(); // Skip '('
                
                // Parse arguments
                let mut arguments = Vec::new();
                
                // Check for empty argument list
                if let Some(TokenType::RightParen) = &self.current_token {
                    self.advance(); // Skip ')'
                    current_object = Expression::MethodCall(
                        Box::new(current_object), 
                        method_name, 
                        arguments
                    );
                    continue;
                }
                
                // Parse first argument
                arguments.push(self.parse_expression());
                
                // Parse remaining arguments
                while let Some(TokenType::Comma) = &self.current_token {
                    self.advance(); // Skip ','
                    arguments.push(self.parse_expression());
                }
                
                // Expect ')'
                if let Some(TokenType::RightParen) = &self.current_token {
                    self.advance(); // Skip ')'
                } else {
                    panic!("Expected ')' after method arguments");
                }
                
                current_object = Expression::MethodCall(
                    Box::new(current_object), 
                    method_name, 
                    arguments
                );
            } else {
                panic!("Expected '(' after method name");
            }
        }
        
        current_object
    }

    fn parse_array_literal(&mut self) -> Expression {
        self.advance(); // Skip '['
        
        let mut elements = Vec::new();
        
        // Check for empty array
        if let Some(TokenType::RightBracket) = &self.current_token {
            self.advance(); // Skip ']'
            return Expression::ArrayLiteral(elements);
        }
        
        // Parse first element
        elements.push(self.parse_expression());
        
        // Parse remaining elements
        while let Some(TokenType::Comma) = &self.current_token {
            self.advance(); // Skip ','
            elements.push(self.parse_expression());
        }
        
        // Expect ']'
        if let Some(TokenType::RightBracket) = &self.current_token {
            self.advance(); // Skip ']'
        } else {
            panic!("Expected ']' after array elements");
        }
        
        Expression::ArrayLiteral(elements)
    }

    fn parse_print_statement(&mut self) -> Result<ASTNode, RavenError> {
        self.advance(); // Skip 'print'
    
        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance(); // Skip '('
    
            // Parse arguments like a function call
            let mut arguments = Vec::new();
            
            // Check for empty argument list
            if let Some(TokenType::RightParen) = &self.current_token {
                self.advance(); // Skip ')'
            } else {
                // Parse first argument
                arguments.push(self.parse_expression());
                
                // Parse remaining arguments
                while let Some(TokenType::Comma) = &self.current_token {
                    self.advance(); // Skip ','
                    arguments.push(self.parse_expression());
                }
                
                // Expect ')'
                if let Some(TokenType::RightParen) = &self.current_token {
                    self.advance(); // Skip ')'
                } else {
                    let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(parse_error("Expected ')' after print arguments", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Close the parenthesis with ')'".to_string()));
                }
            }
    
            if let Some(TokenType::Semicolon) = &self.current_token {
                self.advance(); // Skip ';'
                
                // Return as function call for built-in handling
                return Ok(ASTNode::FunctionCall("print".to_string(), arguments));
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected ';' after print statement", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Add ';' at the end".to_string())
                );
            }
        }
    
        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
        Err(parse_error("Expected '(' after 'print'", span)
            .with_source(self.source_code.clone())
            .with_hint("Use: print(expression);".to_string()))
    }

    fn parse_function_declaration(&mut self) -> Result<ASTNode, RavenError> {
        self.advance(); // Skip 'fun'
    
        // Parse function name
        let func_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone = name.clone();
            self.advance();
            name_clone
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected function name after 'fun'", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Provide a function name".to_string())
            );
        };
    
        // Expect '('
        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected '(' after function name", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add '(' to start parameter list".to_string())
            );
        }
    
        // Parse parameters
        let mut parameters = Vec::new();
        while let Some(token) = &self.current_token {
            if let TokenType::RightParen = token {
                break;
            }
    
            // Parse parameter name
            let param_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                name_clone
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected parameter name", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Provide a parameter name".to_string())
                );
            };
    
            // Expect ':'
            if let Some(TokenType::Colon) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected ':' after parameter name", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Add ':' followed by the parameter type".to_string())
                );
            }
    
            // Parse parameter type
            let param_type = match &self.current_token {
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
                _ => {
                    let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(
                        parse_error("Expected type for parameter", span)
                            .with_source(self.source_code.clone())
                            .with_hint("Use: int, float, bool, or string".to_string())
                    );
                }
            };
    
            parameters.push(crate::ast::Parameter {
                name: param_name,
                param_type,
            });
    
            // Check for comma or end of parameters
            if let Some(TokenType::Comma) = &self.current_token {
                self.advance();
            }
        }
    
        // Expect ')'
        if let Some(TokenType::RightParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected ')' after parameters", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Close the parameter list with ')'".to_string())
            );
        }
    
        // Parse return type
        let return_type = if let Some(TokenType::Arrow) = &self.current_token {
            self.advance();
            match &self.current_token {
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
                _ => {
                    let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(
                        parse_error("Expected return type", span)
                            .with_source(self.source_code.clone())
                            .with_hint("Use: int, float, bool, string, or void".to_string())
                    );
                }
            }
        } else {
            "void".to_string()
        };
    
        // Parse function body
        if let Some(TokenType::LeftBrace) = &self.current_token {
            self.advance();
            let body = self.parse_block()?;
            
            if let Some(TokenType::RightBrace) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected '}' to close function body", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Add '}' to close the function body".to_string())
                );
            }
    
            Ok(ASTNode::FunctionDecl(func_name, return_type, parameters, Box::new(body)))
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            Err(
                parse_error("Expected '{' to start function body", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add '{' to start the function body".to_string())
            )
        }
    }

    fn parse_while_loop(&mut self) -> Result<ASTNode, RavenError> {
        self.advance(); // Skip 'while'
    
        // Expect '('
        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected '(' after 'while'", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Use: while (condition) { ... }".to_string())
            );
        }
    
        // Parse condition
        let condition = self.parse_expression();
    
        // Expect ')'
        if let Some(TokenType::RightParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected ')' after while condition", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Close the condition with ')'".to_string())
            );
        }
    
        // Parse body
        if let Some(TokenType::LeftBrace) = &self.current_token {
            self.advance();
            let body = self.parse_block()?;
            
            if let Some(TokenType::RightBrace) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected '}' to close while body", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Add '}' to close the loop body".to_string())
                );
            }
    
            Ok(ASTNode::WhileLoop(Box::new(condition), Box::new(body)))
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            Err(
                parse_error("Expected '{' to start while body", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add '{' after the condition".to_string())
            )
        }
    }

    fn parse_for_loop(&mut self) -> Result<ASTNode, RavenError> {
        self.advance(); // Skip 'for'
    
        // Expect '('
        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected '(' after 'for'", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Use: for (let i: int = 0; i < 10; i = i + 1) { ... }".to_string())
            );
        }
    
        // Parse initialization (e.g., let i = 0)
        let init = if let Some(TokenType::Let) = &self.current_token {
            self.parse_variable_declaration()?
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected variable declaration in for loop initialization", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Use 'let' to declare the loop variable".to_string())
            );
        };
    
        // Parse condition (e.g., i < 10)
        let condition = self.parse_expression();
    
        // Expect ';'
        if let Some(TokenType::Semicolon) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected ';' after for loop condition", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add ';' after the condition".to_string())
            );
        }
    
        // Parse increment (e.g., i = i + 1) - without semicolon
        let increment = if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone = name.clone();
            self.advance();
            
            if let Some(TokenType::Assign) = &self.current_token {
                self.advance();
                let expr = self.parse_expression();
                ASTNode::Assignment(name_clone, Box::new(expr))
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected '=' in for loop increment", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Use: i = i + 1".to_string())
                );
            }
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected assignment in for loop increment", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Provide an assignment like: i = i + 1".to_string())
            );
        };
    
        // Expect ')'
        if let Some(TokenType::RightParen) = &self.current_token {
            self.advance();
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            return Err(
                parse_error("Expected ')' after for loop header", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Close the for loop header with ')'".to_string())
            );
        }
    
        // Parse body
        if let Some(TokenType::LeftBrace) = &self.current_token {
            self.advance();
            let body = self.parse_block()?;
            
            if let Some(TokenType::RightBrace) = &self.current_token {
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(
                    parse_error("Expected '}' to close for body", span)
                        .with_source(self.source_code.clone())
                        .with_hint("Add '}' to close the loop body".to_string())
                );
            }
    
            Ok(ASTNode::ForLoop(Box::new(init), Box::new(condition), Box::new(increment), Box::new(body)))
        } else {
            let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
            Err(
                parse_error("Expected '{' to start for body", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add '{' after the for loop header".to_string())
            )
        }
    }

    fn parse_return_statement(&mut self) -> Result<ASTNode, RavenError> {
        self.advance(); // Skip 'return'
    
        let expr_start_line = self.lexer.line;
        let expr = self.parse_expression();
    
        if let Some(TokenType::Semicolon) = &self.current_token {
            self.advance();
            Ok(ASTNode::Return(Box::new(expr)))
        } else {
            // Same logic for accurate line numbers
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
            Err(
                parse_error("Expected ';' after return statement", span)
                    .with_source(self.source_code.clone())
                    .with_hint("Add ';' at the end of the statement".to_string())
            )
        }
    }
    
    fn parse_import_statement(&mut self) -> Result<ASTNode, RavenError> {
        self.advance(); // Skip 'import'
        
        // Check for selective import: import { item1, item2 } from "module"
        if let Some(TokenType::LeftBrace) = &self.current_token {
            self.advance(); // Skip '{'
            
            let mut items = Vec::new();
            
            // Parse first item
            if let Some(TokenType::Identifier(item)) = &self.current_token {
                items.push(item.clone());
                self.advance();
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected identifier in import list", span)
                    .with_source(self.source_code.clone()));
            }
            
            // Parse remaining items
            while let Some(TokenType::Comma) = &self.current_token {
                self.advance(); // Skip ','
                
                if let Some(TokenType::Identifier(item)) = &self.current_token {
                    items.push(item.clone());
                    self.advance();
                } else {
                    let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                    return Err(parse_error("Expected identifier after comma in import list", span)
                        .with_source(self.source_code.clone()));
                }
            }
            
            // Expect '}'
            if let Some(TokenType::RightBrace) = &self.current_token {
                self.advance(); // Skip '}'
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected '}' after import list", span)
                    .with_source(self.source_code.clone()));
            }
            
            // Expect 'from'
            if let Some(TokenType::From) = &self.current_token {
                self.advance(); // Skip 'from'
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected 'from' after import list", span)
                    .with_source(self.source_code.clone()));
            }
            
            // Expect module name (string literal)
            let module_name = if let Some(TokenType::StringLiteral(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                name_clone
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected module name (string) after 'from'", span)
                    .with_source(self.source_code.clone()));
            };
            
            // Expect semicolon
            if let Some(TokenType::Semicolon) = &self.current_token {
                self.advance(); // Skip ';'
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ';' after import statement", span)
                    .with_source(self.source_code.clone()));
            }
            
            Ok(ASTNode::ImportSelective(module_name, items))
        } else {
            // Regular import: import module_name from "module" or import "module"
            let module_name = if let Some(TokenType::StringLiteral(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                name_clone
            } else if let Some(TokenType::Identifier(name)) = &self.current_token {
                let name_clone = name.clone();
                self.advance();
                
                // Check for 'from' keyword
                if let Some(TokenType::From) = &self.current_token {
                    self.advance(); // Skip 'from'
                    
                    // Expect module path
                    let module_path = if let Some(TokenType::StringLiteral(path)) = &self.current_token {
                        let path_clone = path.clone();
                        self.advance();
                        path_clone
                    } else {
                        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected module path (string) after 'from'", span)
                            .with_source(self.source_code.clone()));
                    };
                    
                    // Expect semicolon
                    if let Some(TokenType::Semicolon) = &self.current_token {
                        self.advance(); // Skip ';'
                    } else {
                        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ';' after import statement", span)
                            .with_source(self.source_code.clone()));
                    }
                    
                    return Ok(ASTNode::Import(module_path, Some(name_clone)));
                } else {
                    // Direct import without 'from'
                    if let Some(TokenType::Semicolon) = &self.current_token {
                        self.advance(); // Skip ';'
                    } else {
                        let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                        return Err(parse_error("Expected ';' after import statement", span)
                            .with_source(self.source_code.clone()));
                    }
                    
                    return Ok(ASTNode::Import(name_clone, None));
                }
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected module name or identifier after 'import'", span)
                    .with_source(self.source_code.clone()));
            };
            
            // Direct string import
            if let Some(TokenType::Semicolon) = &self.current_token {
                self.advance(); // Skip ';'
            } else {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected ';' after import statement", span)
                    .with_source(self.source_code.clone()));
            }
            
            Ok(ASTNode::Import(module_name, None))
        }
    }
    
    fn parse_export_statement(&mut self) -> Result<ASTNode, RavenError> {
        self.advance(); // Skip 'export'
        
        // Parse the statement to export
        let stmt = match &self.current_token {
            Some(TokenType::Let) => self.parse_variable_declaration()?,
            Some(TokenType::Fun) => {
                // For function declarations, we need to parse them directly
                // since the function parser expects to start with 'fun'
                self.parse_function_declaration()?
            },
            _ => {
                let span = Span::new(self.lexer.line, self.lexer.column, self.lexer.position, 1);
                return Err(parse_error("Expected 'let' or 'fun' after 'export'", span)
                    .with_source(self.source_code.clone()));
            }
        };
        
        Ok(ASTNode::Export(Box::new(stmt)))
    }
    
    
}
