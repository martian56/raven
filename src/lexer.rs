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
    Export,
    From,
    Struct,
    Print,

    // Types
    IntType,
    FloatType,
    BoolType,
    StringType,
    VoidType,
    
    // Array tokens
    LeftBracket,  // [
    RightBracket, // ]

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
    Dot,         // .
    LeftParen,   // (
    RightParen,  // )
    LeftBrace,   // {
    RightBrace,  // }
    Arrow,       // ->
    Ampersand,   // &
    Bang,        // !
    Question,    // ?
    Tilde,       // ~
    Backslash,   // \
    At,          // @
    Dollar,      // $
    Hash,        // #

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

use crate::span::Span;

#[derive(Debug, Clone)]
pub struct Lexer {
    input: Vec<char>,
    pub position: usize,
    current_char: Option<char>,
    pub line: usize,
    pub column: usize,
    line_start: usize,  // Offset where current line starts
}

impl Lexer {
    pub fn new(input: String) -> Self {
        let chars: Vec<char> = input.chars().collect();
        let first_char: Option<char> = chars.get(0).cloned();
        Lexer {
            input: chars,
            position: 0,
            current_char: first_char,
            line: 0,
            column: 0,
            line_start: 0,
        }
    }
    
    /// Get current span for a token starting at current position
    fn current_span(&self, length: usize) -> Span {
        Span::new(self.line, self.column, self.position, length)
    }
    
    /// Mark the start of a token
    fn mark_start(&self) -> (usize, usize, usize) {
        (self.position, self.line, self.column)
    }
    
    /// Create span from marked start to current position
    fn span_from(&self, start: (usize, usize, usize)) -> Span {
        let (start_pos, start_line, start_col) = start;
        Span::new(start_line, start_col, start_pos, self.position - start_pos)
    }

    /// Moves to the next character in input
    pub fn advance(&mut self) {
        // Check if current character is newline before moving
        if let Some('\n') = self.current_char {
            self.line += 1;
            self.column = 0;
            self.line_start = self.position + 1;
        } else {
            self.column += 1;
        }
        
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
    
    pub fn peek_token(&self) -> Option<TokenType> {
        // Create a temporary lexer to peek at the next token
        let mut temp_lexer = self.clone();
        temp_lexer.position = self.position;
        temp_lexer.current_char = self.current_char;
        temp_lexer.line = self.line;
        temp_lexer.column = self.column;
        temp_lexer.line_start = self.line_start;
        
        // Get the next token without advancing the main lexer
        Some(temp_lexer.next_token())
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

    /// Skips a single-line comment (//)
    fn skip_single_line_comment(&mut self) {
        // Skip the two slashes
        self.advance(); // Skip first '/'
        self.advance(); // Skip second '/'
        
        // Skip until end of line
        while let Some(ch) = self.current_char {
            if ch == '\n' {
                self.advance(); // Skip the newline
                break;
            }
            self.advance();
        }
    }

    /// Skips a multi-line comment (/* */)
    fn skip_multi_line_comment(&mut self) {
        // Skip the opening /*
        self.advance(); // Skip '/'
        self.advance(); // Skip '*'
        
        // Skip until closing */
        while let Some(ch) = self.current_char {
            if ch == '*' {
                self.advance(); // Skip '*'
                if let Some('/') = self.current_char {
                    self.advance(); // Skip '/'
                    break;
                }
            } else {
                self.advance();
            }
        }
    }

    pub fn next_token(&mut self) -> TokenType {
        self.skip_whitespace();

        // Handle comments
        if let Some('/') = self.current_char {
            if let Some('/') = self.peek() {
                // Single-line comment: //
                self.skip_single_line_comment();
                return self.next_token(); // Recursively get next token
            } else if let Some('*') = self.peek() {
                // Multi-line comment: /*
                self.skip_multi_line_comment();
                return self.next_token(); // Recursively get next token
            }
        }

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
                    '[' => { self.advance(); TokenType::LeftBracket }
                    ']' => { self.advance(); TokenType::RightBracket }
                    '&' => {
                        if self.peek() == Some('&') {
                            self.advance();
                            self.advance();
                            TokenType::And
                        } else {
                            self.advance();
                            TokenType::Ampersand
                        }
                    }
                    '|' => {
                        if self.peek() == Some('|') {
                            self.advance();
                            self.advance();
                            TokenType::Or
                        } else {
                            self.advance();
                            TokenType::Illegal('|') // or define a single '|' token if needed
                        }
                    }
                    '.' => {
                        if self.peek() == Some('.') {
                            self.advance();
                            self.advance();
                            TokenType::DotDot
                        } else {
                            self.advance();
                            TokenType::Dot
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
            "export" => TokenType::Export,
            "from" => TokenType::From,
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