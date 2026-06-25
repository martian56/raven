//! Recursive descent parser for the v2 grammar.
//!
//! The parser consumes a token slice produced by `crate::lexer` and
//! emits an AST (`crate::ast::File`). It is hand written and single pass.
//! `parse_file` stops at the first error; `parse_file_recover` recovers at
//! item boundaries to report several syntax errors per compile.
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
    /// The file's macro definitions, used to expand a macro call that
    /// appears inside a `"${...}"` interpolation fragment (which is lexed
    /// here during parsing, after the main pre-pass). Empty when the file
    /// defines no macros.
    pub(crate) macros: crate::macros::MacroTable,
    /// Error-recovery mode. When set (by `parse_file_recover`), a failed
    /// statement inside a block is recorded and recovered to the next
    /// statement boundary instead of aborting the parse, so several syntax
    /// errors are reported per compile. `parse_file` leaves this off and
    /// stops at the first error.
    recovering: bool,
    /// Diagnostics accumulated during recovery (statement and item level).
    errors: Vec<RavenError>,
    /// Log of in-place token rewrites (currently only `consume_close_angle`
    /// splitting a `>>` into `>`), as `(index, original token)`. `rewind`
    /// replays it so a failed speculative parse does not leave a split `>>`
    /// behind.
    token_edits: Vec<(usize, Token)>,
    /// Definition-site identifiers a macro expansion in a `"${...}"`
    /// interpolation fragment introduced. Collected here and handed back by
    /// `parse_with_macros`/`parse_with_macros_all` so the resolver resolves them
    /// at the macro's module scope, not against a call-site local.
    def_sites: crate::macros::DefSites,
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
            macros: crate::macros::MacroTable::default(),
            recovering: false,
            errors: Vec::new(),
            token_edits: Vec::new(),
            def_sites: crate::macros::DefSites::new(),
        }
    }

    /// Build a parser that knows the file's macro definitions, so a macro
    /// call inside a `"${...}"` interpolation fragment can be expanded while
    /// the fragment is parsed. Equivalent to [`Parser::new`] when the table
    /// is empty.
    pub fn new_with_macros(tokens: &[Token], macros: crate::macros::MacroTable) -> Self {
        Parser {
            tokens: tokens.to_vec(),
            pos: 0,
            no_struct_literal: false,
            macros,
            recovering: false,
            errors: Vec::new(),
            token_edits: Vec::new(),
            def_sites: crate::macros::DefSites::new(),
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

    /// Parse the entire token stream, recovering at item boundaries so a
    /// syntax error in one top-level item does not hide errors in the next.
    /// On a failed `parse_decl`, the error is recorded and the parser skips
    /// to the next item-starting keyword before continuing. Returns every
    /// collected parse error when any occurred (the partial item list is
    /// discarded, since a damaged AST would only produce cascade errors
    /// downstream).
    pub fn parse_file_recover(&mut self) -> Result<File, Vec<RavenError>> {
        self.recovering = true;
        let start_span = self.peek().span.clone();
        let mut items = Vec::new();
        self.skip_separators();
        while !self.is_at_end() {
            match self.parse_decl() {
                Ok(item) => items.push(item),
                Err(e) => {
                    self.errors.push(e);
                    self.recover_to_next_item();
                }
            }
            self.skip_separators();
        }
        // The sink holds both item-level errors (above) and any
        // statement-level errors recovered inside block bodies.
        let mut errors = std::mem::take(&mut self.errors);
        dedup_parse_errors(&mut errors);
        if errors.is_empty() {
            let end_span = self.peek().span.clone();
            Ok(File {
                items,
                span: merge_spans(&start_span, &end_span),
            })
        } else {
            Err(errors)
        }
    }

    /// Skip tokens after a failed item parse until the next item-starting
    /// keyword at the top level. Always consumes at least one token so
    /// recovery makes progress, then tracks bracket depth and stops at the
    /// first unambiguous top-level item keyword (`fun`, `struct`, `enum`,
    /// `trait`, `impl`, `extern`, `import`, or a `@` attribute). `let` and
    /// `const` are not sync points because they double as statements inside a
    /// body.
    fn recover_to_next_item(&mut self) {
        if !self.is_at_end() {
            self.advance();
        }
        let mut depth: i32 = 0;
        while !self.is_at_end() {
            let k = self.peek_kind();
            if depth <= 0 && is_item_start(k) {
                break;
            }
            match k {
                TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => depth += 1,
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => depth -= 1,
                _ => {}
            }
            self.advance();
        }
    }

    /// Skip tokens after a failed statement parse until the next statement
    /// boundary inside the current block: a top-level newline or `;` (where
    /// the next statement begins), or the block's own closing `}` (where the
    /// block ends). Consumes at least one token for progress, and tracks
    /// bracket depth so a separator or `}` inside a nested group does not end
    /// recovery early. Leaves the boundary token in place for the block loop.
    fn recover_to_next_stmt(&mut self) {
        if !matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof) {
            self.advance();
        }
        let mut depth: i32 = 0;
        loop {
            match self.peek_kind() {
                TokenKind::Eof => break,
                TokenKind::RBrace if depth == 0 => break,
                TokenKind::Newline | TokenKind::Semi if depth == 0 => break,
                TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => {
                    depth += 1;
                    self.advance();
                }
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => {
                    depth -= 1;
                    self.advance();
                }
                _ => {
                    self.advance();
                }
            }
        }
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
        // Undo any in-place token rewrite at or after the rewind point so a
        // failed speculative parse does not leave a `>>` split into a single
        // `>`. Edits before the rewind point were part of already-committed
        // parsing and stay.
        let mut i = 0;
        while i < self.token_edits.len() {
            if self.token_edits[i].0 >= pos {
                let (idx, original) = self.token_edits.remove(i);
                self.tokens[idx] = original;
            } else {
                i += 1;
            }
        }
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

/// Parse a complete source file, carrying the file's macro definitions so a
/// macro call inside a `"${...}"` interpolation fragment expands.
pub fn parse_with_macros(
    tokens: &[Token],
    macros: crate::macros::MacroTable,
) -> ParseResult<(File, crate::macros::DefSites)> {
    let mut p = Parser::new_with_macros(tokens, macros);
    let file = p.parse_file()?;
    Ok((file, p.def_sites))
}

/// Parse a complete source file with item-level error recovery, carrying the
/// file's macro definitions. Returns every collected parse error when any
/// occurred, so one compile reports several syntax errors.
pub fn parse_with_macros_all(
    tokens: &[Token],
    macros: crate::macros::MacroTable,
) -> Result<(File, crate::macros::DefSites), Vec<RavenError>> {
    let mut p = Parser::new_with_macros(tokens, macros);
    let file = p.parse_file_recover()?;
    Ok((file, p.def_sites))
}

/// Whether `k` begins an unambiguous top-level item, used as a parser
/// synchronization point during error recovery. `let` and `const` are
/// excluded because they also appear as statements inside a body.
fn is_item_start(k: &TokenKind) -> bool {
    matches!(
        k,
        TokenKind::Fun
            | TokenKind::Struct
            | TokenKind::Enum
            | TokenKind::Trait
            | TokenKind::Impl
            | TokenKind::Extern
            | TokenKind::Import
            | TokenKind::At
    )
}

/// Drop duplicate parse diagnostics, keeping the first occurrence. Two errors
/// at the same span with the same message are duplicates.
fn dedup_parse_errors(errors: &mut Vec<RavenError>) {
    let mut seen: std::collections::HashSet<(std::path::PathBuf, usize, usize, String)> =
        std::collections::HashSet::new();
    errors.retain(|e| {
        let s = e.span();
        seen.insert((s.file.to_path_buf(), s.start, s.end, format!("{}", e)))
    });
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
        TokenKind::IntMinMagnitude => "integer `9223372036854775808`".to_string(),
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
        TokenKind::Spawn => "`spawn`".to_string(),
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
        TokenKind::DotDotDot => "`...`".to_string(),
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
