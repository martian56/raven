use crate::lexer::{Lexer, TokenType};
use crate::ast::{ASTNode, Expression, Operator};

pub struct Parser {
    lexer: Lexer,
    current_token: Option<TokenType>,
}

impl Parser {
    pub fn new(lexer: Lexer) -> Self {
        let mut parser: Parser = Parser {
            lexer,
            current_token: None,
        };
        parser.advance();
        parser
    }

    fn advance(&mut self) {
        self.current_token = Some(self.lexer.next_token());
    }

    pub fn parse(&mut self) -> Result<ASTNode, String> {
        let mut statements: Vec<ASTNode> = Vec::new();
    
        while let Some(token) = &self.current_token {
            let stmt: ASTNode = match token {
                TokenType::Let => self.parse_variable_declaration()?,
                TokenType::Identifier(_) => self.parse_assignment()?,
                TokenType::If => self.parse_if_statement()?,
                TokenType::Print => self.parse_print_statement()?,
                TokenType::EOF => break,
                _ => return Err(format!("Unexpected token: {:?}", token)),
            };
    
            statements.push(stmt);
            // self.advance(); ❌ remove this!
        }
    
        Ok(ASTNode::Block(statements))
    }
    
    
    fn parse_variable_declaration(&mut self) -> Result<ASTNode, String> {
        self.advance(); // Skip 'let'
    
        // Step 1: Expect identifier
        let var_name = if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone = name.clone();
            self.advance(); // Skip identifier
            name_clone
        } else {
            return Err("Expected identifier after 'let'.".to_string());
        };
    
        // Step 2: Expect colon
        if let Some(TokenType::Colon) = &self.current_token {
            self.advance(); // Skip ':'
        } else {
            return Err("Expected ':' after variable name.".to_string());
        }
    
        // Step 3: Expect a type (IntType, FloatType, etc.)
        let var_type = match &self.current_token {
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
            other => return Err(format!("Expected type after ':', got {:?}", other)),
        };
    
        // Step 4: Expect '='
        if let Some(TokenType::Assign) = &self.current_token {
            self.advance(); // Skip '='
        } else {
            return Err("Expected '=' after variable type.".to_string());
        }
    
        // Step 5: Parse the expression
        let expr = self.parse_expression();
    
        // Step 6: Expect semicolon
        if let Some(TokenType::Semicolon) = &self.current_token {
            self.advance(); // Skip ';'
        } else {
            return Err("Expected ';' after variable declaration.".to_string());
        }
    
        Ok(ASTNode::VariableDeclTyped(var_name, var_type, Box::new(expr)))
    }
    

    fn parse_assignment(&mut self) -> Result<ASTNode, String> {
        if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone: String = name.clone();
            self.advance(); // Skip identifier

            if let Some(TokenType::Assign) = &self.current_token {
                self.advance(); // Skip '='
                let expr: Expression = self.parse_expression();

                if let Some(TokenType::Semicolon) = &self.current_token {
                    self.advance(); // Skip ';'
                    return Ok(ASTNode::Assignment(name_clone, Box::new(expr)));
                } else {
                    return Err("Expected ';' after assignment.".to_string());
                }
            }
        }

        Err("Invalid assignment.".to_string())
    }

    fn parse_if_statement(&mut self) -> Result<ASTNode, String> {
        self.parse_if_like_statement(true)
    }
    
    fn parse_if_like_statement(&mut self, is_if: bool) -> Result<ASTNode, String> {
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
                                    return Err("Expected '{' after 'else'.".to_string());
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
                    return Err("Expected '{' to start if block.".to_string());
                }
            } else {
                return Err("Expected ')' after condition.".to_string());
            }
        }
    
        Err(format!("Expected '(' after '{}'.", if is_if { "if" } else { "elseif" }))
    }
    


    fn parse_block(&mut self) -> Result<ASTNode, String> {
        let mut statements = Vec::new();
    
        while let Some(token) = &self.current_token {
            if let TokenType::RightBrace = token {
                break; // Stop before consuming the closing brace
            }
    
            let stmt = match token {
                TokenType::Let => self.parse_variable_declaration()?,
                TokenType::Identifier(_) => self.parse_assignment()?,
                TokenType::If => self.parse_if_statement()?,
                TokenType::Print => self.parse_print_statement()?,
                _ => return Err(format!("Unexpected token in block: {:?}", token)),
            };
    
            statements.push(stmt);
        }
    
        Ok(ASTNode::Block(statements))
    }
    

    fn parse_expression(&mut self) -> Expression {
        let mut left: Expression = self.parse_term();

        while let Some(op) = self.match_operator() {
            self.advance(); // Skip operator
            let right: Expression = self.parse_term();
            left = Expression::BinaryOp(Box::new(left), op, Box::new(right));
        }

        left
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
            Some(TokenType::Identifier(name)) => {
                let name_clone = name.clone();
                self.advance();
                Expression::Identifier(name_clone)
            }
            _ => panic!("Unexpected token in expression: {:?}", self.current_token),
        }
    }

    fn parse_print_statement(&mut self) -> Result<ASTNode, String> {
        self.advance(); // Skip 'print'
    
        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance(); // Skip '('
    
            let expr = self.parse_expression();
    
            if let Some(TokenType::RightParen) = &self.current_token {
                self.advance(); // Skip ')'
    
                if let Some(TokenType::Semicolon) = &self.current_token {
                    self.advance(); // Skip ';'
                    return Ok(ASTNode::Print(Box::new(expr)));
                } else {
                    return Err("Expected ';' after print statement.".to_string());
                }
            } else {
                return Err("Expected ')' after expression in print.".to_string());
            }
        }
    
        Err("Expected '(' after 'print'.".to_string())
    }
    
    
}
