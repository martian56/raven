//! Recursive descent parser for the v2 grammar.
//!
//! The parser consumes a token slice produced by `crate::lexer` and
//! emits an AST (`crate::ast::File`). It is hand written, single pass,
//! and non recovering: the first error stops parsing.
//!
//! Internally the implementation is split by grammar category:
//!
//! * `expr`: value expressions and operator precedence
//! * `stmt`: statements inside blocks
//! * `decl`: top level items
//! * `ty`: type expressions
//! * `pattern`: match and binding patterns
//!
//! The cursor and helper machinery live in this module on the `Parser`
//! struct; the category modules implement methods on it.
//!
//! See `docs/v2/specs/parser.md` for the full grammar and the design
//! notes on disambiguation rules.

use crate::ast::File;
use crate::error::{ParseError, RavenError};
use crate::lexer::{Token, TokenKind};
use crate::span::Span;

mod decl;
mod expr;
mod pattern;
mod stmt;
mod ty;

#[cfg(test)]
mod tests;

/// Convenience alias for parser results.
pub type ParseResult<T> = Result<T, RavenError>;

/// The recursive descent parser state.
///
/// The parser owns its token buffer (cloned from the input slice) so
/// that nested generic angle brackets can be rewritten on the fly: a
/// trailing `>>` is split into two `>` tokens when the inner type
/// argument list closes. Without owning the buffer, that rewrite would
/// require unsafe.
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// Suppresses parsing a `{` as a struct literal when set. Toggled by
    /// the `if` / `while` / `for` / `match` heads while parsing the
    /// condition or scrutinee expression. See the parser spec.
    pub(crate) no_struct_literal: bool,
}

impl Parser {
    /// Build a new parser over the given token slice. The slice must
    /// end with `TokenKind::Eof`, as produced by `Lexer::tokenize`. The
    /// parser clones the tokens internally so the caller's buffer can
    /// be freed.
    pub fn new(tokens: &[Token]) -> Self {
        Parser {
            tokens: tokens.to_vec(),
            pos: 0,
            no_struct_literal: false,
        }
    }

    /// Parse the entire token stream into a [`File`].
    pub fn parse_file(&mut self) -> ParseResult<File> {
        let start_span = self.peek().span.clone();
        let mut items = Vec::new();
        self.skip_separators();
        while !self.is_at_end() {
            let item = self.parse_decl()?;
            items.push(item);
            // Items are separated by newlines, semicolons, or both.
            // Trailing separators are fine.
            self.skip_separators();
        }
        let end_span = self.peek().span.clone();
        Ok(File {
            items,
            span: merge_spans(&start_span, &end_span),
        })
    }

    // ----- token cursor primitives -----

    /// The current token without consuming.
    pub(crate) fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    /// The token `offset` positions ahead of the cursor (0 == current).
    pub(crate) fn peek_at(&self, offset: usize) -> &Token {
        let i = self.pos.saturating_add(offset).min(self.tokens.len() - 1);
        &self.tokens[i]
    }

    /// Kind of the current token, by reference.
    pub(crate) fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    /// Kind of the token `offset` ahead of the cursor.
    pub(crate) fn peek_kind_at(&self, offset: usize) -> &TokenKind {
        &self.peek_at(offset).kind
    }

    /// True at the EOF sentinel.
    pub(crate) fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    /// Consume and return the current token.
    pub(crate) fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if !matches!(t.kind, TokenKind::Eof) {
            self.pos += 1;
        }
        t
    }

    /// Save the cursor position so a trial parse can rewind.
    pub(crate) fn checkpoint(&self) -> usize {
        self.pos
    }

    /// Rewind to a previously saved cursor.
    pub(crate) fn rewind(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// If the current token matches `kind`, consume it and return true.
    pub(crate) fn eat(&mut self, kind: &TokenKind) -> bool {
        if std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Consume the current token if it matches `kind`, or raise an
    /// "expected X" error.
    pub(crate) fn expect(&mut self, kind: &TokenKind, expected_name: &str) -> ParseResult<Token> {
        if std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind) {
            Ok(self.advance())
        } else {
            Err(self.unexpected(expected_name))
        }
    }

    /// Consume the current token, requiring it to be an identifier, and
    /// return the identifier name plus its span.
    pub(crate) fn expect_ident(&mut self, ctx: &str) -> ParseResult<(String, Span)> {
        let tok = self.peek().clone();
        if let TokenKind::Identifier(name) = tok.kind.clone() {
            self.advance();
            Ok((name, tok.span))
        } else {
            Err(self.unexpected(ctx))
        }
    }

    /// Build an "unexpected token" error referencing the current token.
    pub(crate) fn unexpected(&self, expected: &str) -> RavenError {
        let tok = self.peek();
        if matches!(tok.kind, TokenKind::Eof) {
            RavenError::parse(
                ParseError::UnexpectedEof {
                    expected: expected.to_string(),
                },
                tok.span.clone(),
            )
        } else {
            RavenError::parse(
                ParseError::UnexpectedToken {
                    expected: expected.to_string(),
                    found: describe_token(&tok.kind),
                },
                tok.span.clone(),
            )
        }
    }

    // ----- separator handling -----

    /// Skip any run of `Newline` and `Semi` tokens. Used between items,
    /// between statements, after `,` in flexible separator contexts.
    pub(crate) fn skip_separators(&mut self) {
        while matches!(self.peek_kind(), TokenKind::Newline | TokenKind::Semi) {
            self.advance();
        }
    }

    /// Skip any run of `Newline` tokens only. Used inside expressions
    /// at continuation points.
    pub(crate) fn skip_newlines(&mut self) {
        while matches!(self.peek_kind(), TokenKind::Newline) {
            self.advance();
        }
    }
}

/// Parse a complete source file from its token slice.
pub fn parse(tokens: &[Token]) -> ParseResult<File> {
    let mut p = Parser::new(tokens);
    p.parse_file()
}

/// Merge two spans into one covering both. Both must come from the same
/// source file.
pub(crate) fn merge_spans(a: &Span, b: &Span) -> Span {
    Span::new(
        a.file.clone(),
        a.start.min(b.start),
        a.end.max(b.end),
        a.line,
        a.col,
    )
}

/// Human readable label for a token kind, used in error messages.
pub(crate) fn describe_token(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Identifier(s) => format!("identifier `{}`", s),
        TokenKind::IntLit(n) => format!("integer `{}`", n),
        TokenKind::FloatLit(n) => format!("float `{}`", n),
        TokenKind::StringLit(_) => "string literal".to_string(),
        TokenKind::BlockStringLit(_) => "block string literal".to_string(),
        TokenKind::CharLit(_) => "char literal".to_string(),
        TokenKind::CStringLit(_) => "c-string literal".to_string(),
        TokenKind::Let => "`let`".to_string(),
        TokenKind::Const => "`const`".to_string(),
        TokenKind::Fun => "`fun`".to_string(),
        TokenKind::Return => "`return`".to_string(),
        TokenKind::If => "`if`".to_string(),
        TokenKind::Else => "`else`".to_string(),
        TokenKind::While => "`while`".to_string(),
        TokenKind::For => "`for`".to_string(),
        TokenKind::Loop => "`loop`".to_string(),
        TokenKind::In => "`in`".to_string(),
        TokenKind::Break => "`break`".to_string(),
        TokenKind::Continue => "`continue`".to_string(),
        TokenKind::Match => "`match`".to_string(),
        TokenKind::Struct => "`struct`".to_string(),
        TokenKind::Trait => "`trait`".to_string(),
        TokenKind::Impl => "`impl`".to_string(),
        TokenKind::Enum => "`enum`".to_string(),
        TokenKind::Import => "`import`".to_string(),
        TokenKind::As => "`as`".to_string(),
        TokenKind::Extern => "`extern`".to_string(),
        TokenKind::Defer => "`defer`".to_string(),
        TokenKind::True => "`true`".to_string(),
        TokenKind::False => "`false`".to_string(),
        TokenKind::SelfLower => "`self`".to_string(),
        TokenKind::SelfUpper => "`Self`".to_string(),
        TokenKind::Plus => "`+`".to_string(),
        TokenKind::Minus => "`-`".to_string(),
        TokenKind::Star => "`*`".to_string(),
        TokenKind::Slash => "`/`".to_string(),
        TokenKind::Percent => "`%`".to_string(),
        TokenKind::PlusEq => "`+=`".to_string(),
        TokenKind::MinusEq => "`-=`".to_string(),
        TokenKind::StarEq => "`*=`".to_string(),
        TokenKind::SlashEq => "`/=`".to_string(),
        TokenKind::PercentEq => "`%=`".to_string(),
        TokenKind::EqEq => "`==`".to_string(),
        TokenKind::NotEq => "`!=`".to_string(),
        TokenKind::Lt => "`<`".to_string(),
        TokenKind::Gt => "`>`".to_string(),
        TokenKind::LtEq => "`<=`".to_string(),
        TokenKind::GtEq => "`>=`".to_string(),
        TokenKind::AndAnd => "`&&`".to_string(),
        TokenKind::OrOr => "`||`".to_string(),
        TokenKind::Bang => "`!`".to_string(),
        TokenKind::Amp => "`&`".to_string(),
        TokenKind::Pipe => "`|`".to_string(),
        TokenKind::Caret => "`^`".to_string(),
        TokenKind::Tilde => "`~`".to_string(),
        TokenKind::Shl => "`<<`".to_string(),
        TokenKind::Shr => "`>>`".to_string(),
        TokenKind::AmpEq => "`&=`".to_string(),
        TokenKind::PipeEq => "`|=`".to_string(),
        TokenKind::CaretEq => "`^=`".to_string(),
        TokenKind::ShlEq => "`<<=`".to_string(),
        TokenKind::ShrEq => "`>>=`".to_string(),
        TokenKind::Eq => "`=`".to_string(),
        TokenKind::DotDot => "`..`".to_string(),
        TokenKind::DotDotEq => "`..=`".to_string(),
        TokenKind::Question => "`?`".to_string(),
        TokenKind::Arrow => "`->`".to_string(),
        TokenKind::FatArrow => "`=>`".to_string(),
        TokenKind::ColonColon => "`::`".to_string(),
        TokenKind::Dot => "`.`".to_string(),
        TokenKind::LParen => "`(`".to_string(),
        TokenKind::RParen => "`)`".to_string(),
        TokenKind::LBrace => "`{`".to_string(),
        TokenKind::RBrace => "`}`".to_string(),
        TokenKind::LBracket => "`[`".to_string(),
        TokenKind::RBracket => "`]`".to_string(),
        TokenKind::Comma => "`,`".to_string(),
        TokenKind::Semi => "`;`".to_string(),
        TokenKind::Colon => "`:`".to_string(),
        TokenKind::At => "`@`".to_string(),
        TokenKind::Dollar => "`$`".to_string(),
        TokenKind::Newline => "newline".to_string(),
        TokenKind::Eof => "end of file".to_string(),
    }
}
