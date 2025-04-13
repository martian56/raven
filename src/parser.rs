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
                TokenType::EOF => break, // Add this line to break on EOF
                _ => return Err(format!("Unexpected token: {:?}", token)),
            };
    
            statements.push(stmt);
            self.advance();
        }
    
        Ok(ASTNode::Block(statements))
    }
    

    // pub fn parse(&mut self) -> Result<ASTNode, String> {
    //     let mut statements: Vec<ASTNode> = Vec::new();
    
    //     loop {
    //         match &self.current_token {
    //             Some(TokenType::EOF) => break,
    //             Some(TokenType::Let) => {
    //                 let stmt: ASTNode = self.parse_variable_declaration()?;
    //                 statements.push(stmt);
    //             }
    //             Some(TokenType::Identifier(_)) => {
    //                 let stmt: ASTNode = self.parse_assignment()?;
    //                 statements.push(stmt);
    //             }
    //             Some(TokenType::If) => {
    //                 let stmt: ASTNode = self.parse_if_statement()?;
    //                 statements.push(stmt);
    //             }
    //             Some(token) => {
    //                 return Err(format!("Unexpected token: {:?}", token));
    //             }
    //             None => {
    //                 return Err("Unexpected end of input".to_string());
    //             }
    //         }
    //     }
    
    //     Ok(ASTNode::Block(statements))
    // }
    

    fn parse_variable_declaration(&mut self) -> Result<ASTNode, String> {
        self.advance(); // Skip "let"

        if let Some(TokenType::Identifier(name)) = &self.current_token {
            let name_clone: String = name.clone();
            self.advance(); // Skip the identifier

            if let Some(TokenType::Assign) = &self.current_token {
                self.advance(); // Skip '='
                let expr: Expression = self.parse_expression();

                if let Some(TokenType::Semicolon) = &self.current_token {
                    self.advance(); // Skip ';'
                    return Ok(ASTNode::VariableDecl(name_clone, Box::new(expr)));
                } else {
                    return Err("Expected ';' after variable declaration.".to_string());
                }
            } else {
                return Err("Expected '=' after variable name.".to_string());
            }
        }

        Err("Expected identifier after 'let'.".to_string())
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
        self.advance(); // Skip 'if'

        if let Some(TokenType::LeftParen) = &self.current_token {
            self.advance(); // Skip '('
            let condition: Expression = self.parse_expression();

            if let Some(TokenType::RightParen) = &self.current_token {
                self.advance(); // Skip ')'

                if let Some(TokenType::LeftBrace) = &self.current_token {
                    self.advance(); // Skip '{'
                    let body: ASTNode = self.parse()?; // Recursive parse to handle block

                    if let Some(TokenType::RightBrace) = &self.current_token {
                        self.advance(); // Skip '}'
                        return Ok(ASTNode::IfStatement(Box::new(condition), Box::new(body)));
                    } else {
                        return Err("Expected '}' after if block.".to_string());
                    }
                } else {
                    return Err("Expected '{' to start if block.".to_string());
                }
            } else {
                return Err("Expected ')' after if condition.".to_string());
            }
        }

        Err("Expected '(' after 'if'.".to_string())
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
            _ => None,
        }
    }

    fn parse_term(&mut self) -> Expression {
        match &self.current_token {
            Some(TokenType::Integer(value)) => {
                let val: i64 = *value;
                self.advance();
                Expression::Integer(val)
            }
            Some(TokenType::Identifier(name)) => {
                let name_clone: String = name.clone();
                self.advance();
                Expression::Identifier(name_clone)
            }
            Some(TokenType::StringLiteral(s)) => {
                let s_clone: String = s.clone();
                self.advance();
                Expression::StringLiteral(s_clone)
            }
            _ => panic!("Unexpected token in expression: {:?}", self.current_token),
        }
    }
}
