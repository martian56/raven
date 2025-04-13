#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    // Keywords
    Let,
    Const,
    Fun,
    Return,
    If,
    ElseIf,
    Else,
    While,
    For,
    Import,
    Struct,
    Print,

    // Types
    IntType,
    FloatType,
    BoolType,
    StringType,
    VoidType,

    // Identifiers and literals
    Integer(i64),
    Identifier(String),
    IntLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    BoolLiteral(bool),

    // Symbols
    Assign,      // =
    Colon,       // :
    Semicolon,   // ;
    Comma,       // ,
    LeftParen,   // (
    RightParen,  // )
    LeftBrace,   // {
    RightBrace,  // }
    Arrow,       // ->

    // Operators
    Plus,        // +
    Minus,       // -
    Star,        // *
    Slash,       // /
    Percent,     // %

    // Comparison
    EqualEqual,      // ==
    NotEqual,        // !=
    Less,            // <
    Greater,         // >
    LessEqual,       // <=
    GreaterEqual,    // >=

    // Logical
    And,       // &&
    Or,        // ||
    Not,       // !

    // Range
    DotDot,    // ..

    EOF,
    Illegal(char),
}


pub struct Lexer {
    input: Vec<char>,
    position: usize,
    current_char: Option<char>,
}

impl Lexer {
    pub fn new(input: String) -> Self {
        let chars: Vec<char> = input.chars().collect();
        let first_char: Option<char> = chars.get(0).cloned();
        Lexer {
            input: chars,
            position: 0,
            current_char: first_char,
        }
    }

    /// Moves to the next character in input
    pub fn advance(&mut self) {
        self.position += 1;
        if self.position >= self.input.len() {
            self.current_char = None;
        } else {
            self.current_char = Some(self.input[self.position]);
        }
    }

    /// Peeks at the next character without consuming it
    pub fn peek(&self) -> Option<char> {
        if self.position + 1 >= self.input.len() {
            None
        } else {
            Some(self.input[self.position + 1])
        }
    }

    /// Skips whitespace (spaces, tabs, newlines)
    pub fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Reads an identifier or keyword (e.g., let, fun, name, foo)
    pub fn read_identifier(&mut self) -> String {
        let mut result: String = String::new();
        while let Some(ch) = self.current_char {
            if ch.is_alphanumeric() || ch == '_' {
                result.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        result
    }

    /// Reads a number (int or float)
    pub fn read_number(&mut self) -> String {
        let mut result: String = String::new();
        let mut has_dot: bool = false;

        while let Some(ch) = self.current_char {
            if ch.is_digit(10) {
                result.push(ch);
            } else if ch == '.' && !has_dot {
                has_dot = true;
                result.push(ch);
            } else {
                break;
            }
            self.advance();
        }

        result
    }

    /// Reads a string literal like "hello world"
    pub fn read_string(&mut self) -> String {
        let mut result: String = String::new();
        self.advance(); // Skip opening quote

        while let Some(ch) = self.current_char {
            if ch == '"' {
                break;
            } else {
                result.push(ch);
                self.advance();
            }
        }

        self.advance(); // Skip closing quote
        result
    }

    pub fn next_token(&mut self) -> TokenType {
        self.skip_whitespace();

        match self.current_char {
            Some(ch) => {
                // Handle single-character tokens
                match ch {
                    '=' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            self.advance();
                            TokenType::EqualEqual
                        } else {
                            self.advance();
                            TokenType::Assign
                        }
                    }
                    '!' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            self.advance();
                            TokenType::NotEqual
                        } else {
                            self.advance();
                            TokenType::Not
                        }
                    }
                    '<' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            self.advance();
                            TokenType::LessEqual
                        } else {
                            self.advance();
                            TokenType::Less
                        }
                    }
                    '>' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            self.advance();
                            TokenType::GreaterEqual
                        } else {
                            self.advance();
                            TokenType::Greater
                        }
                    }
                    ':' => { self.advance(); TokenType::Colon }
                    ';' => { self.advance(); TokenType::Semicolon }
                    ',' => { self.advance(); TokenType::Comma }
                    '+' => { self.advance(); TokenType::Plus }
                    '-' => {
                        if self.peek() == Some('>') {
                            self.advance();
                            self.advance();
                            TokenType::Arrow
                        } else {
                            self.advance();
                            TokenType::Minus
                        }
                    }
                    '*' => { self.advance(); TokenType::Star }
                    '/' => { self.advance(); TokenType::Slash }
                    '%' => { self.advance(); TokenType::Percent }
                    '(' => { self.advance(); TokenType::LeftParen }
                    ')' => { self.advance(); TokenType::RightParen }
                    '{' => { self.advance(); TokenType::LeftBrace }
                    '}' => { self.advance(); TokenType::RightBrace }
                    '.' => {
                        if self.peek() == Some('.') {
                            self.advance();
                            self.advance();
                            TokenType::DotDot
                        } else {
                            self.advance();
                            TokenType::Illegal('.')
                        }
                    }
                    '"' => {
                        let string: String = self.read_string();
                        TokenType::StringLiteral(string)
                    }
                    ch if ch.is_ascii_digit() => {
                        let number: String = self.read_number();
                        if number.contains('.') {
                            TokenType::FloatLiteral(number.parse::<f64>().unwrap())
                        } else {
                            TokenType::IntLiteral(number.parse::<i64>().unwrap())
                        }
                    }
                    ch if ch.is_ascii_alphabetic() || ch == '_' => {
                        let ident: String = self.read_identifier();
                        self.lookup_keyword_or_identifier(&ident)
                    }
                    _ => {
                        let illegal: char = ch;
                        self.advance();
                        TokenType::Illegal(illegal)
                    }
                }
            }
            None => TokenType::EOF,
        }
    }

    fn lookup_keyword_or_identifier(&self, ident: &str) -> TokenType {
        match ident {
            "let" => TokenType::Let,
            "const" => TokenType::Const,
            "fun" => TokenType::Fun,
            "return" => TokenType::Return,
            "if" => TokenType::If,
            "elseif" => TokenType::ElseIf,
            "else" => TokenType::Else,
            "while" => TokenType::While,
            "for" => TokenType::For,
            "import" => TokenType::Import,
            "struct" => TokenType::Struct,
            "print" => TokenType::Print,
            "and" => TokenType::And,
            "or" => TokenType::Or,
            "not" => TokenType::Not,

            // types
            "int" => TokenType::IntType,
            "float" => TokenType::FloatType,
            "bool" => TokenType::BoolType,
            "String" => TokenType::StringType,
            "void" => TokenType::VoidType,

            // literals
            "true" => TokenType::BoolLiteral(true),
            "false" => TokenType::BoolLiteral(false),

            _ => TokenType::Identifier(ident.to_string()),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexer() {
        let input: String = "let x = 5;".to_string();
        let mut lexer: Lexer = Lexer::new(input);
        assert_eq!(lexer.next_token(), TokenType::Let);
        assert_eq!(lexer.next_token(), TokenType::Identifier("x".to_string()));
        assert_eq!(lexer.next_token(), TokenType::Assign);
        assert_eq!(lexer.next_token(), TokenType::IntLiteral(5));
        assert_eq!(lexer.next_token(), TokenType::Semicolon);
        assert_eq!(lexer.next_token(), TokenType::EOF);
    }

    #[test]
    fn test_read_identifier() {
        let input: String = "let x = 5;".to_string();
        let mut lexer: Lexer = Lexer::new(input);
        let identifier: String = lexer.read_identifier();
        assert_eq!(identifier, "let");
    }

    #[test]
    fn test_read_number() {
        let input: String = "let x = 5.5;".to_string();
        let mut lexer: Lexer = Lexer::new(input);
        lexer.advance(); // Skip 'l'
        lexer.advance(); // Skip 'e'
        lexer.advance(); // Skip 't'
        lexer.advance(); // Skip ' '
        lexer.advance(); // Skip 'x'
        lexer.advance(); // Skip ' '
        lexer.advance(); // Skip '='
        lexer.advance(); // Skip ' '
        let number: String = lexer.read_number();
        assert_eq!(number, "5.5");
    }

    #[test]
    fn test_read_string() {
        let input: String = r#"let x = "hello";"#.to_string();
        let mut lexer: Lexer = Lexer::new(input);
        lexer.advance(); // Skip 'l'
        lexer.advance(); // Skip 'e'
        lexer.advance(); // Skip 't'
        lexer.advance(); // Skip ' '
        lexer.advance(); // Skip 'x'
        lexer.advance(); // Skip ' '
        lexer.advance(); // Skip '='
        lexer.advance(); // Skip ' '
        let string: String = lexer.read_string();
        println!("String: {}", string);
        assert_eq!(string, "hello");
    }

    #[test]
    fn test_lookup_keyword_or_identifier() {
        let input: String = "let".to_string();
        let lexer: Lexer = Lexer::new(input);
        let token: TokenType = lexer.lookup_keyword_or_identifier("let");
        assert_eq!(token, TokenType::Let);
    }
}