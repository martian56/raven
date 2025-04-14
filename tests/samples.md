
## We have a parse method example

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

## Parse_Variable_declaration without type (this will not be used)

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