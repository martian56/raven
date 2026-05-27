//! Type expression parsing.

use crate::ast::{Type, TypeKind, TypePath, TypePathSegment};
use crate::lexer::{Token, TokenKind};
use crate::span::Span;

use super::{merge_spans, ParseResult, Parser};

impl Parser {
    /// Parse a `Type`. Handles `T?` sugar, `dyn Trait`, `()`, and
    /// `fun(...) -> T`.
    pub(crate) fn parse_type(&mut self) -> ParseResult<Type> {
        let primary = self.parse_primary_type()?;
        if matches!(self.peek_kind(), TokenKind::Question) {
            let q = self.advance();
            let span = merge_spans(&primary.span, &q.span);
            return Ok(Type {
                kind: TypeKind::Optional(Box::new(primary)),
                span,
            });
        }
        Ok(primary)
    }

    fn parse_primary_type(&mut self) -> ParseResult<Type> {
        let start = self.peek().span.clone();
        match self.peek_kind() {
            TokenKind::LParen => {
                self.advance(); // (
                let rparen = self.expect(&TokenKind::RParen, "`)`")?;
                let span = merge_spans(&start, &rparen.span);
                Ok(Type {
                    kind: TypeKind::Unit,
                    span,
                })
            }
            TokenKind::Identifier(name) if name == "dyn" => {
                // `dyn` is a contextual keyword: an identifier
                // lexically, a type keyword in this position.
                self.advance();
                let path = self.parse_type_path()?;
                let span = merge_spans(&start, &path.span);
                Ok(Type {
                    kind: TypeKind::Dyn(path),
                    span,
                })
            }
            TokenKind::Fun => {
                self.advance();
                self.expect(&TokenKind::LParen, "`(`")?;
                let mut params = Vec::new();
                if !matches!(self.peek_kind(), TokenKind::RParen) {
                    loop {
                        self.skip_newlines();
                        params.push(self.parse_type()?);
                        self.skip_newlines();
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                        self.skip_newlines();
                        if matches!(self.peek_kind(), TokenKind::RParen) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "`)`")?;
                self.expect(&TokenKind::Arrow, "`->`")?;
                let ret = self.parse_type()?;
                let span = merge_spans(&start, &ret.span);
                Ok(Type {
                    kind: TypeKind::Function {
                        params,
                        ret: Box::new(ret),
                    },
                    span,
                })
            }
            TokenKind::Identifier(_) | TokenKind::SelfUpper => {
                let path = self.parse_type_path()?;
                let span = path.span.clone();
                Ok(Type {
                    kind: TypeKind::Path(path),
                    span,
                })
            }
            _ => Err(self.unexpected("type")),
        }
    }

    /// Parse a dot separated qualified type path with optional generic
    /// arguments at each segment.
    pub(crate) fn parse_type_path(&mut self) -> ParseResult<TypePath> {
        let first = self.parse_type_path_segment()?;
        let mut span = first.span.clone();
        let mut segments = vec![first];
        while matches!(self.peek_kind(), TokenKind::Dot)
            && matches!(self.peek_kind_at(1), TokenKind::Identifier(_))
        {
            self.advance(); // .
            let seg = self.parse_type_path_segment()?;
            span = merge_spans(&span, &seg.span);
            segments.push(seg);
        }
        Ok(TypePath { segments, span })
    }

    fn parse_type_path_segment(&mut self) -> ParseResult<TypePathSegment> {
        let tok = self.peek().clone();
        let (name, name_span) = match tok.kind {
            TokenKind::Identifier(n) => {
                self.advance();
                (n, tok.span)
            }
            TokenKind::SelfUpper => {
                self.advance();
                ("Self".to_string(), tok.span)
            }
            _ => return Err(self.unexpected("type name")),
        };
        let mut span = name_span;
        let mut generics = Vec::new();
        if matches!(self.peek_kind(), TokenKind::Lt) {
            let (args, args_span) = self.parse_type_args_required()?;
            generics = args;
            span = merge_spans(&span, &args_span);
        }
        Ok(TypePathSegment {
            name,
            generics,
            span,
        })
    }

    /// Parse a required `<T, U, ...>` block. The caller has verified
    /// the leading `<` is present.
    pub(crate) fn parse_type_args_required(&mut self) -> ParseResult<(Vec<Type>, Span)> {
        let start = self.peek().span.clone();
        self.advance(); // <
        let mut args = Vec::new();
        if !matches!(self.peek_kind(), TokenKind::Gt | TokenKind::Shr) {
            loop {
                self.skip_newlines();
                args.push(self.parse_type()?);
                self.skip_newlines();
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
                if matches!(self.peek_kind(), TokenKind::Gt | TokenKind::Shr) {
                    break;
                }
            }
        }
        // Close angle: a single `>` is consumed normally; a trailing
        // `>>` is split into two close angles for nested generics
        // like `Vec<Vec<Int>>`.
        let close_span = self.consume_close_angle()?;
        Ok((args, merge_spans(&start, &close_span)))
    }

    /// Consume one closing `>`. If the current token is `>>`, split it
    /// into two `>` tokens, consume the first half, and leave the
    /// second half in place for the outer call.
    fn consume_close_angle(&mut self) -> ParseResult<Span> {
        match self.peek_kind() {
            TokenKind::Gt => Ok(self.advance().span),
            TokenKind::Shr => {
                let tok = self.tokens[self.pos].clone();
                let first_half = Span::new(
                    tok.span.file.clone(),
                    tok.span.start,
                    tok.span.start + 1,
                    tok.span.line,
                    tok.span.col,
                );
                let second_half = Span::new(
                    tok.span.file.clone(),
                    tok.span.start + 1,
                    tok.span.end,
                    tok.span.line,
                    tok.span.col.saturating_add(1),
                );
                // Replace the `>>` token in place with a single `>`
                // representing the second half. The cursor stays at
                // the same position so the outer call sees that `>`.
                self.tokens[self.pos] = Token {
                    kind: TokenKind::Gt,
                    span: second_half,
                };
                Ok(first_half)
            }
            _ => Err(self.unexpected("`>`")),
        }
    }
}
