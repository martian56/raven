//! Raven v2 lexer.
//!
//! Tokenizes UTF-8 source text into a `Vec<Token>` ending in `TokenKind::Eof`.
//! See `docs/v2/specs/lexer.md` for the full token catalog,
//! conventions, and edge cases.
//!
//! The lexer owns its source as a `String` (not a `&str`) so that the
//! returned `Token` values, which carry `Span`s referencing this source, can
//! outlive the lexer without lifetime gymnastics. Source is expected to be
//! valid UTF-8 (Rust strings are guaranteed so).

use std::path::PathBuf;
use std::sync::Arc;

use crate::error::{LexError, RavenError};
use crate::span::Span;

/// Private-use sentinel the lexer emits in front of an escaped dollar
/// sign (`\$`). String interpolation splitting runs on the lexer's
/// decoded literal text, where an escaped `\$` and a real `$` would
/// otherwise be indistinguishable. The sentinel marks the dollar as
/// escaped; the interpolation splitter strips it and treats the dollar
/// as ordinary text. `U+E000` is in the Unicode private-use area and is
/// reserved for this internal purpose. See
/// `docs/v2/specs/interpolation.md`.
pub const ESCAPED_DOLLAR_SENTINEL: char = '\u{E000}';

/// Kinds of tokens produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals and identifiers.
    Identifier(String),
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    BlockStringLit(String),
    CharLit(char),
    CStringLit(String),

    // Keywords.
    Let,
    Const,
    Fun,
    Return,
    If,
    Else,
    While,
    For,
    Loop,
    In,
    Break,
    Continue,
    Match,
    Struct,
    Trait,
    Impl,
    Enum,
    Import,
    As,
    Extern,
    Defer,
    Spawn,
    True,
    False,
    /// lowercase `self`
    SelfLower,
    /// uppercase `Self`
    SelfUpper,

    // Arithmetic.
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,

    // Comparison.
    EqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,

    // Boolean.
    AndAnd,
    OrOr,
    Bang,

    // Bitwise.
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,

    // Assignment and structure.
    Eq,
    DotDot,
    DotDotEq,
    /// `...`, the C variadic parameter marker in an `extern` signature.
    DotDotDot,
    Question,
    Arrow,    // ->
    FatArrow, // =>
    ColonColon,
    Dot,

    // Punctuation.
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    At,
    /// Metavariable sigil in declarative macros (`$name`). Outside a macro
    /// definition or invocation it has no meaning and the parser rejects it.
    Dollar,

    // Whitespace significant.
    Newline,
    Eof,
}

/// A token plus its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Token { kind, span }
    }
}

/// The lexer state.
pub struct Lexer {
    source: String,
    /// Byte offset into `source`.
    pos: usize,
    /// 1 indexed current line.
    line: u32,
    /// 1 indexed current column (counted in chars).
    col: u32,
    /// Shared owner of the source file path for `Span`s.
    file: Arc<PathBuf>,
}

impl Lexer {
    /// Build a lexer over `source` annotated as coming from `file`.
    pub fn new(source: impl Into<String>, file: impl Into<PathBuf>) -> Self {
        Lexer {
            source: source.into(),
            pos: 0,
            line: 1,
            col: 1,
            file: Arc::new(file.into()),
        }
    }

    /// Consume the entire input and return all tokens including the final
    /// `Eof`.
    pub fn tokenize(&mut self) -> Result<Vec<Token>, RavenError> {
        let mut out = Vec::new();
        loop {
            match self.next_token()? {
                Some(t) => {
                    let is_eof = matches!(t.kind, TokenKind::Eof);
                    out.push(t);
                    if is_eof {
                        break;
                    }
                }
                None => continue,
            }
        }
        Ok(out)
    }

    /// Produce the next token, or `Ok(None)` if the current step consumed
    /// whitespace or a comment and the caller should retry.
    pub fn next_token(&mut self) -> Result<Option<Token>, RavenError> {
        // Skip non newline whitespace and comments.
        loop {
            match self.peek() {
                Some(' ') | Some('\t') => {
                    self.bump();
                }
                Some('/') if self.peek_at(1) == Some('/') => {
                    self.consume_line_comment();
                }
                Some('/') if self.peek_at(1) == Some('*') => {
                    self.consume_block_comment()?;
                }
                _ => break,
            }
        }

        let start = self.pos;
        let line = self.line;
        let col = self.col;

        let Some(ch) = self.peek() else {
            // EOF: a zero width span at the current position.
            let span = Span::point(self.file.clone(), self.pos, self.line, self.col);
            return Ok(Some(Token::new(TokenKind::Eof, span)));
        };

        // Newline run: collapse into a single Newline token.
        if ch == '\n' || ch == '\r' {
            self.consume_newlines();
            let span = self.make_span(start, line, col);
            return Ok(Some(Token::new(TokenKind::Newline, span)));
        }

        // Numeric literal.
        if ch.is_ascii_digit() {
            return self.lex_number(start, line, col).map(Some);
        }

        // Identifiers and keywords, with the c"..." prefix special case.
        if ch == 'c' && self.peek_at(1) == Some('"') {
            self.bump(); // consume 'c'
            return self
                .lex_string_after_quote(start, line, col, true)
                .map(Some);
        }
        if is_ident_start(ch) {
            return self.lex_identifier_or_keyword(start, line, col).map(Some);
        }

        // String literals.
        if ch == '"' {
            return self
                .lex_string_after_quote(start, line, col, false)
                .map(Some);
        }

        // Char literals.
        if ch == '\'' {
            return self.lex_char(start, line, col).map(Some);
        }

        // Operators and punctuation.
        self.lex_operator(start, line, col).map(Some)
    }

    // ----- helpers -----

    fn peek(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        let mut chars = self.source[self.pos..].chars();
        for _ in 0..offset {
            chars.next();
        }
        chars.next()
    }

    /// Consume one character and advance line/col. Treats `\r\n` and bare
    /// `\r` as one newline.
    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        let len = ch.len_utf8();
        self.pos += len;
        if ch == '\r' {
            if self.peek() == Some('\n') {
                self.pos += 1;
            }
            self.line += 1;
            self.col = 1;
        } else if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    fn make_span(&self, start: usize, line: u32, col: u32) -> Span {
        Span::new(self.file.clone(), start, self.pos, line, col)
    }

    fn err(&self, kind: LexError, start: usize, line: u32, col: u32) -> RavenError {
        RavenError::lex(kind, self.make_span(start, line, col))
    }

    fn consume_line_comment(&mut self) {
        // `//` has been peeked, not consumed.
        self.bump();
        self.bump();
        while let Some(ch) = self.peek() {
            if ch == '\n' || ch == '\r' {
                break;
            }
            self.bump();
        }
    }

    fn consume_block_comment(&mut self) -> Result<(), RavenError> {
        let start = self.pos;
        let line = self.line;
        let col = self.col;
        self.bump(); // /
        self.bump(); // *
        loop {
            match self.peek() {
                None => {
                    return Err(self.err(LexError::UnterminatedBlockComment, start, line, col));
                }
                Some('*') if self.peek_at(1) == Some('/') => {
                    self.bump();
                    self.bump();
                    return Ok(());
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    fn consume_newlines(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == '\n' || ch == '\r' {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn lex_identifier_or_keyword(
        &mut self,
        start: usize,
        line: u32,
        col: u32,
    ) -> Result<Token, RavenError> {
        while let Some(ch) = self.peek() {
            if is_ident_continue(ch) {
                self.bump();
            } else {
                break;
            }
        }
        let lexeme = &self.source[start..self.pos];
        let kind = match lexeme {
            "let" => TokenKind::Let,
            "const" => TokenKind::Const,
            "fun" => TokenKind::Fun,
            "return" => TokenKind::Return,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "loop" => TokenKind::Loop,
            "in" => TokenKind::In,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "match" => TokenKind::Match,
            "struct" => TokenKind::Struct,
            "trait" => TokenKind::Trait,
            "impl" => TokenKind::Impl,
            "enum" => TokenKind::Enum,
            "import" => TokenKind::Import,
            "as" => TokenKind::As,
            "extern" => TokenKind::Extern,
            "defer" => TokenKind::Defer,
            "spawn" => TokenKind::Spawn,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "self" => TokenKind::SelfLower,
            "Self" => TokenKind::SelfUpper,
            _ => TokenKind::Identifier(lexeme.to_string()),
        };
        Ok(Token::new(kind, self.make_span(start, line, col)))
    }

    fn lex_number(&mut self, start: usize, line: u32, col: u32) -> Result<Token, RavenError> {
        // Detect base prefixes.
        let first = self.peek().unwrap();
        if first == '0' {
            if let Some(next) = self.peek_at(1) {
                match next {
                    'x' | 'X' => {
                        self.bump();
                        self.bump();
                        return self.lex_int_with_radix(start, line, col, 16);
                    }
                    'b' | 'B' => {
                        self.bump();
                        self.bump();
                        return self.lex_int_with_radix(start, line, col, 2);
                    }
                    'o' | 'O' => {
                        self.bump();
                        self.bump();
                        return self.lex_int_with_radix(start, line, col, 8);
                    }
                    _ => {}
                }
            }
        }

        // Decimal integer or float.
        let digits_start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() || ch == '_' {
                self.bump();
            } else {
                break;
            }
        }

        let mut is_float = false;

        // Fractional part: must be `.` followed by a digit (not `..` range
        // and not `.method` field access).
        if self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            self.bump(); // .
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() || ch == '_' {
                    self.bump();
                } else {
                    break;
                }
            }
        }

        // Exponent.
        if matches!(self.peek(), Some('e') | Some('E')) {
            is_float = true;
            self.bump();
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.bump();
            }
            let exp_digit_start = self.pos;
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() || ch == '_' {
                    self.bump();
                } else {
                    break;
                }
            }
            if self.pos == exp_digit_start {
                let lexeme = self.source[start..self.pos].to_string();
                return Err(self.err(LexError::InvalidNumber(lexeme), start, line, col));
            }
        }

        let lexeme = &self.source[start..self.pos];
        let cleaned: String = lexeme.chars().filter(|c| *c != '_').collect();

        if is_float {
            match cleaned.parse::<f64>() {
                Ok(v) => Ok(Token::new(
                    TokenKind::FloatLit(v),
                    self.make_span(start, line, col),
                )),
                Err(_) => Err(self.err(LexError::InvalidNumber(lexeme.into()), start, line, col)),
            }
        } else {
            let _ = digits_start; // suppress unused warning in the float path
            match cleaned.parse::<i64>() {
                Ok(v) => Ok(Token::new(
                    TokenKind::IntLit(v),
                    self.make_span(start, line, col),
                )),
                Err(_) => Err(self.err(LexError::InvalidNumber(lexeme.into()), start, line, col)),
            }
        }
    }

    fn lex_int_with_radix(
        &mut self,
        start: usize,
        line: u32,
        col: u32,
        radix: u32,
    ) -> Result<Token, RavenError> {
        let digits_start = self.pos;
        while let Some(ch) = self.peek() {
            if ch == '_' || ch.is_digit(radix) {
                self.bump();
            } else {
                break;
            }
        }
        if self.pos == digits_start {
            let lexeme = self.source[start..self.pos].to_string();
            return Err(self.err(LexError::InvalidNumber(lexeme), start, line, col));
        }
        let raw = &self.source[digits_start..self.pos];
        let cleaned: String = raw.chars().filter(|c| *c != '_').collect();
        match i64::from_str_radix(&cleaned, radix) {
            Ok(v) => Ok(Token::new(
                TokenKind::IntLit(v),
                self.make_span(start, line, col),
            )),
            Err(_) => {
                let lexeme = self.source[start..self.pos].to_string();
                Err(self.err(LexError::InvalidNumber(lexeme), start, line, col))
            }
        }
    }

    /// Lex a `"..."` string. `start`, `line`, `col` point at the leading `c`
    /// for a CString literal, or at the opening `"` otherwise. The opening
    /// `"` is at `self.pos` on entry (the `c` has already been consumed for
    /// `is_cstring`).
    fn lex_string_after_quote(
        &mut self,
        start: usize,
        line: u32,
        col: u32,
        is_cstring: bool,
    ) -> Result<Token, RavenError> {
        // Check for triple quoted block string (only without c prefix).
        if !is_cstring
            && self.peek() == Some('"')
            && self.peek_at(1) == Some('"')
            && self.peek_at(2) == Some('"')
        {
            self.bump();
            self.bump();
            self.bump();
            return self.lex_block_string(start, line, col);
        }

        // Single line string.
        self.bump(); // opening "
        let mut out = String::new();
        // Brace depth inside a `${...}` interpolation. While greater than 0,
        // the bytes are an embedded expression copied verbatim (no escape
        // decoding) for the parser's interpolation splitter to re-lex, and a
        // `"` opens a nested string literal rather than closing this one.
        let mut interp_depth: u32 = 0;
        loop {
            match self.peek() {
                None => {
                    return Err(self.err(LexError::UnterminatedString, start, line, col));
                }
                Some('"') if interp_depth == 0 => {
                    self.bump();
                    let span = self.make_span(start, line, col);
                    let kind = if is_cstring {
                        TokenKind::CStringLit(out)
                    } else {
                        TokenKind::StringLit(out)
                    };
                    return Ok(Token::new(kind, span));
                }
                Some('"') => {
                    // A nested string literal inside an interpolation. Copy it
                    // verbatim, including escapes and its closing quote, so the
                    // splitter re-parses it as an ordinary string expression.
                    out.push(self.bump().unwrap()); // opening "
                    loop {
                        match self.peek() {
                            None | Some('\n') => {
                                return Err(self.err(
                                    LexError::UnterminatedString,
                                    start,
                                    line,
                                    col,
                                ));
                            }
                            Some('\\') => {
                                out.push(self.bump().unwrap()); // backslash
                                if self.peek().is_some() {
                                    out.push(self.bump().unwrap()); // escaped char
                                }
                            }
                            Some('"') => {
                                out.push(self.bump().unwrap()); // closing "
                                break;
                            }
                            Some(_) => out.push(self.bump().unwrap()),
                        }
                    }
                }
                Some('\n') => {
                    return Err(self.err(LexError::UnterminatedString, start, line, col));
                }
                Some('$') if self.peek_at(1) == Some('{') => {
                    out.push(self.bump().unwrap()); // $
                    out.push(self.bump().unwrap()); // {
                    interp_depth += 1;
                }
                Some('{') if interp_depth > 0 => {
                    interp_depth += 1;
                    out.push(self.bump().unwrap());
                }
                Some('}') if interp_depth > 0 => {
                    interp_depth -= 1;
                    out.push(self.bump().unwrap());
                }
                // Inside an interpolation the bytes are raw expression source,
                // copied verbatim; escape decoding only applies to literal text.
                Some('\\') if interp_depth == 0 => {
                    let esc_line = self.line;
                    let esc_col = self.col;
                    let esc_start = self.pos;
                    self.bump(); // backslash
                    let escaped = self.read_escape(esc_start, esc_line, esc_col)?;
                    out.push_str(&escaped);
                }
                Some(_) => {
                    let ch = self.bump().unwrap();
                    out.push(ch);
                }
            }
        }
    }

    fn lex_block_string(&mut self, start: usize, line: u32, col: u32) -> Result<Token, RavenError> {
        // Triple quote already consumed. Read raw content until closing triple quote.
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(self.err(LexError::UnterminatedBlockString, start, line, col));
                }
                Some('"') if self.peek_at(1) == Some('"') && self.peek_at(2) == Some('"') => {
                    self.bump();
                    self.bump();
                    self.bump();
                    return Ok(Token::new(
                        TokenKind::BlockStringLit(out),
                        self.make_span(start, line, col),
                    ));
                }
                Some(_) => {
                    let ch = self.bump().unwrap();
                    out.push(ch);
                }
            }
        }
    }

    fn lex_char(&mut self, start: usize, line: u32, col: u32) -> Result<Token, RavenError> {
        self.bump(); // opening '
        let value = match self.peek() {
            None => {
                return Err(self.err(
                    LexError::InvalidCharLit("unterminated".into()),
                    start,
                    line,
                    col,
                ));
            }
            Some('\'') => {
                return Err(self.err(LexError::InvalidCharLit("empty".into()), start, line, col));
            }
            Some('\\') => {
                let esc_line = self.line;
                let esc_col = self.col;
                let esc_start = self.pos;
                self.bump();
                let s = self.read_escape(esc_start, esc_line, esc_col)?;
                let mut chars = s.chars();
                let first = chars.next().ok_or_else(|| {
                    self.err(
                        LexError::InvalidCharLit("empty escape".into()),
                        start,
                        line,
                        col,
                    )
                })?;
                if chars.next().is_some() {
                    return Err(self.err(
                        LexError::InvalidCharLit("multi character escape".into()),
                        start,
                        line,
                        col,
                    ));
                }
                first
            }
            Some(ch) => {
                self.bump();
                ch
            }
        };
        match self.peek() {
            Some('\'') => {
                self.bump();
                Ok(Token::new(
                    TokenKind::CharLit(value),
                    self.make_span(start, line, col),
                ))
            }
            Some(_) => Err(self.err(
                LexError::InvalidCharLit("must contain exactly one character".into()),
                start,
                line,
                col,
            )),
            None => Err(self.err(
                LexError::InvalidCharLit("unterminated".into()),
                start,
                line,
                col,
            )),
        }
    }

    /// Read the body of an escape after the leading backslash has been
    /// consumed. Returns the decoded text (typically one char, but `\u{...}`
    /// may yield multi byte UTF-8). `esc_start`, `esc_line`, `esc_col` are
    /// the position of the backslash for error reporting.
    fn read_escape(
        &mut self,
        esc_start: usize,
        esc_line: u32,
        esc_col: u32,
    ) -> Result<String, RavenError> {
        let Some(ch) = self.peek() else {
            return Err(self.err(LexError::UnterminatedString, esc_start, esc_line, esc_col));
        };
        match ch {
            'n' => {
                self.bump();
                Ok("\n".to_string())
            }
            't' => {
                self.bump();
                Ok("\t".to_string())
            }
            'r' => {
                self.bump();
                Ok("\r".to_string())
            }
            '\\' => {
                self.bump();
                Ok("\\".to_string())
            }
            '"' => {
                self.bump();
                Ok("\"".to_string())
            }
            '\'' => {
                self.bump();
                Ok("'".to_string())
            }
            '$' => {
                // `\$` escapes a dollar sign so that `\${...}` is a literal
                // `${...}` and not an interpolation. The decoded text keeps
                // a private-use sentinel (`U+E000`) in front of the dollar
                // so the interpolation splitter, which runs later on the
                // decoded literal, can tell an escaped `$` apart from a real
                // `${` interpolation start. The sentinel is dropped when the
                // splitter emits the literal text. See
                // `docs/v2/specs/interpolation.md`.
                self.bump();
                Ok(format!("{}$", ESCAPED_DOLLAR_SENTINEL))
            }
            '0' => {
                self.bump();
                Ok("\0".to_string())
            }
            'x' => {
                self.bump();
                let mut hex = String::new();
                for _ in 0..2 {
                    match self.peek() {
                        Some(c) if c.is_ascii_hexdigit() => {
                            hex.push(c);
                            self.bump();
                        }
                        _ => {
                            return Err(self.err(
                                LexError::InvalidUnicodeEscape,
                                esc_start,
                                esc_line,
                                esc_col,
                            ));
                        }
                    }
                }
                let n = u32::from_str_radix(&hex, 16).map_err(|_| {
                    self.err(LexError::InvalidUnicodeEscape, esc_start, esc_line, esc_col)
                })?;
                let c = char::from_u32(n).ok_or_else(|| {
                    self.err(LexError::InvalidUnicodeEscape, esc_start, esc_line, esc_col)
                })?;
                Ok(c.to_string())
            }
            'u' => {
                self.bump();
                if self.peek() != Some('{') {
                    return Err(self.err(
                        LexError::InvalidUnicodeEscape,
                        esc_start,
                        esc_line,
                        esc_col,
                    ));
                }
                self.bump();
                let mut hex = String::new();
                while let Some(c) = self.peek() {
                    if c == '}' {
                        break;
                    }
                    if !c.is_ascii_hexdigit() || hex.len() >= 6 {
                        return Err(self.err(
                            LexError::InvalidUnicodeEscape,
                            esc_start,
                            esc_line,
                            esc_col,
                        ));
                    }
                    hex.push(c);
                    self.bump();
                }
                if self.peek() != Some('}') || hex.is_empty() {
                    return Err(self.err(
                        LexError::InvalidUnicodeEscape,
                        esc_start,
                        esc_line,
                        esc_col,
                    ));
                }
                self.bump(); // }
                let n = u32::from_str_radix(&hex, 16).map_err(|_| {
                    self.err(LexError::InvalidUnicodeEscape, esc_start, esc_line, esc_col)
                })?;
                let c = char::from_u32(n).ok_or_else(|| {
                    self.err(LexError::InvalidUnicodeEscape, esc_start, esc_line, esc_col)
                })?;
                Ok(c.to_string())
            }
            other => {
                self.bump();
                Err(self.err(LexError::InvalidEscape(other), esc_start, esc_line, esc_col))
            }
        }
    }

    fn lex_operator(&mut self, start: usize, line: u32, col: u32) -> Result<Token, RavenError> {
        let ch = self.peek().unwrap();
        let make = |this: &mut Self, kind: TokenKind| -> Result<Token, RavenError> {
            Ok(Token::new(kind, this.make_span(start, line, col)))
        };

        match ch {
            '+' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::PlusEq);
                }
                make(self, TokenKind::Plus)
            }
            '-' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::MinusEq);
                }
                if self.peek() == Some('>') {
                    self.bump();
                    return make(self, TokenKind::Arrow);
                }
                make(self, TokenKind::Minus)
            }
            '*' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::StarEq);
                }
                make(self, TokenKind::Star)
            }
            '/' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::SlashEq);
                }
                make(self, TokenKind::Slash)
            }
            '%' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::PercentEq);
                }
                make(self, TokenKind::Percent)
            }
            '=' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::EqEq);
                }
                if self.peek() == Some('>') {
                    self.bump();
                    return make(self, TokenKind::FatArrow);
                }
                make(self, TokenKind::Eq)
            }
            '!' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::NotEq);
                }
                make(self, TokenKind::Bang)
            }
            '<' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::LtEq);
                }
                if self.peek() == Some('<') {
                    self.bump();
                    if self.peek() == Some('=') {
                        self.bump();
                        return make(self, TokenKind::ShlEq);
                    }
                    return make(self, TokenKind::Shl);
                }
                make(self, TokenKind::Lt)
            }
            '>' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::GtEq);
                }
                if self.peek() == Some('>') {
                    self.bump();
                    if self.peek() == Some('=') {
                        self.bump();
                        return make(self, TokenKind::ShrEq);
                    }
                    return make(self, TokenKind::Shr);
                }
                make(self, TokenKind::Gt)
            }
            '&' => {
                self.bump();
                if self.peek() == Some('&') {
                    self.bump();
                    return make(self, TokenKind::AndAnd);
                }
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::AmpEq);
                }
                make(self, TokenKind::Amp)
            }
            '|' => {
                self.bump();
                if self.peek() == Some('|') {
                    self.bump();
                    return make(self, TokenKind::OrOr);
                }
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::PipeEq);
                }
                make(self, TokenKind::Pipe)
            }
            '^' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    return make(self, TokenKind::CaretEq);
                }
                make(self, TokenKind::Caret)
            }
            '~' => {
                self.bump();
                make(self, TokenKind::Tilde)
            }
            '.' => {
                self.bump();
                if self.peek() == Some('.') {
                    self.bump();
                    if self.peek() == Some('=') {
                        self.bump();
                        return make(self, TokenKind::DotDotEq);
                    }
                    if self.peek() == Some('.') {
                        self.bump();
                        return make(self, TokenKind::DotDotDot);
                    }
                    return make(self, TokenKind::DotDot);
                }
                make(self, TokenKind::Dot)
            }
            ':' => {
                self.bump();
                if self.peek() == Some(':') {
                    self.bump();
                    return make(self, TokenKind::ColonColon);
                }
                make(self, TokenKind::Colon)
            }
            '?' => {
                self.bump();
                make(self, TokenKind::Question)
            }
            '(' => {
                self.bump();
                make(self, TokenKind::LParen)
            }
            ')' => {
                self.bump();
                make(self, TokenKind::RParen)
            }
            '{' => {
                self.bump();
                make(self, TokenKind::LBrace)
            }
            '}' => {
                self.bump();
                make(self, TokenKind::RBrace)
            }
            '[' => {
                self.bump();
                make(self, TokenKind::LBracket)
            }
            ']' => {
                self.bump();
                make(self, TokenKind::RBracket)
            }
            ',' => {
                self.bump();
                make(self, TokenKind::Comma)
            }
            ';' => {
                self.bump();
                make(self, TokenKind::Semi)
            }
            '@' => {
                self.bump();
                make(self, TokenKind::At)
            }
            '$' => {
                self.bump();
                make(self, TokenKind::Dollar)
            }
            other => {
                self.bump();
                Err(self.err(LexError::UnexpectedChar(other), start, line, col))
            }
        }
    }
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests;
