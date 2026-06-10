//! Pattern parsing for `match` arms and `for` heads.

use crate::ast::{FieldPattern, LiteralPattern, Pattern, PatternKind};
use crate::error::ParseError;
use crate::lexer::{TokenKind, ESCAPED_DOLLAR_SENTINEL};

use super::{merge_spans, ParseResult, Parser};

impl Parser {
    /// Parse one pattern.
    pub(crate) fn parse_pattern(&mut self) -> ParseResult<Pattern> {
        let start = self.peek().span.clone();
        match self.peek_kind().clone() {
            TokenKind::Identifier(ref name) if name == "_" => {
                let tok = self.advance();
                Ok(Pattern {
                    kind: PatternKind::Wildcard,
                    span: tok.span,
                })
            }
            TokenKind::IntLit(n) => {
                let lo_tok = self.advance();
                // Range pattern: `lo..hi` or `lo..=hi`.
                if matches!(self.peek_kind(), TokenKind::DotDot | TokenKind::DotDotEq) {
                    let inclusive = matches!(self.peek_kind(), TokenKind::DotDotEq);
                    self.advance();
                    let hi_tok = match self.peek_kind() {
                        TokenKind::IntLit(_) => self.advance(),
                        _ => return Err(self.unexpected("integer literal")),
                    };
                    let hi = match hi_tok.kind {
                        TokenKind::IntLit(v) => v,
                        _ => unreachable!(),
                    };
                    let span = merge_spans(&lo_tok.span, &hi_tok.span);
                    return Ok(Pattern {
                        kind: PatternKind::Range {
                            lo: n,
                            hi,
                            inclusive,
                        },
                        span,
                    });
                }
                Ok(Pattern {
                    kind: PatternKind::Literal(LiteralPattern::Int(n)),
                    span: lo_tok.span,
                })
            }
            TokenKind::FloatLit(v) => {
                let tok = self.advance();
                Ok(Pattern {
                    kind: PatternKind::Literal(LiteralPattern::Float(v)),
                    span: tok.span,
                })
            }
            TokenKind::True => {
                let tok = self.advance();
                Ok(Pattern {
                    kind: PatternKind::Literal(LiteralPattern::Bool(true)),
                    span: tok.span,
                })
            }
            TokenKind::False => {
                let tok = self.advance();
                Ok(Pattern {
                    kind: PatternKind::Literal(LiteralPattern::Bool(false)),
                    span: tok.span,
                })
            }
            TokenKind::StringLit(ref s) => {
                // A pattern's string is matched against a value's bytes, so the
                // escaped-dollar sentinel must be stripped to the plain `$`
                // here; the interpolation splitter that normally does this never
                // runs on a pattern.
                let s = s.replace(ESCAPED_DOLLAR_SENTINEL, "");
                let tok = self.advance();
                Ok(Pattern {
                    kind: PatternKind::Literal(LiteralPattern::String(s)),
                    span: tok.span,
                })
            }
            TokenKind::CharLit(c) => {
                let tok = self.advance();
                Ok(Pattern {
                    kind: PatternKind::Literal(LiteralPattern::Char(c)),
                    span: tok.span,
                })
            }
            TokenKind::Minus => {
                // `-N` literal pattern. Only valid for integer or float.
                let minus = self.advance();
                match self.peek_kind().clone() {
                    TokenKind::IntLit(n) => {
                        let tok = self.advance();
                        let span = merge_spans(&minus.span, &tok.span);
                        Ok(Pattern {
                            kind: PatternKind::Literal(LiteralPattern::Int(-n)),
                            span,
                        })
                    }
                    TokenKind::FloatLit(v) => {
                        let tok = self.advance();
                        let span = merge_spans(&minus.span, &tok.span);
                        Ok(Pattern {
                            kind: PatternKind::Literal(LiteralPattern::Float(-v)),
                            span,
                        })
                    }
                    _ => Err(self.unexpected("numeric literal after `-`")),
                }
            }
            TokenKind::LParen => {
                self.advance(); // (
                let mut elements = Vec::new();
                self.skip_newlines();
                if !matches!(self.peek_kind(), TokenKind::RParen) {
                    loop {
                        self.skip_newlines();
                        elements.push(self.parse_pattern()?);
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
                let rparen = self.expect(&TokenKind::RParen, "`)`")?;
                let span = merge_spans(&start, &rparen.span);
                if elements.len() == 1 {
                    // A single parenthesized pattern is just a grouping.
                    return Ok(elements.into_iter().next().unwrap());
                }
                Ok(Pattern {
                    kind: PatternKind::Tuple {
                        name: None,
                        elements,
                    },
                    span,
                })
            }
            TokenKind::Identifier(_) => {
                let (name, name_span) = self.expect_ident("pattern name")?;
                // `Name(...)` enum tuple variant.
                if matches!(self.peek_kind(), TokenKind::LParen) {
                    self.advance(); // (
                    let mut elements = Vec::new();
                    self.skip_newlines();
                    if !matches!(self.peek_kind(), TokenKind::RParen) {
                        loop {
                            self.skip_newlines();
                            elements.push(self.parse_pattern()?);
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
                    let rparen = self.expect(&TokenKind::RParen, "`)`")?;
                    let span = merge_spans(&name_span, &rparen.span);
                    return Ok(Pattern {
                        kind: PatternKind::Tuple {
                            name: Some(name),
                            elements,
                        },
                        span,
                    });
                }
                // `Name { ... }` struct or struct enum variant.
                if matches!(self.peek_kind(), TokenKind::LBrace) {
                    self.advance(); // {
                    let mut fields = Vec::new();
                    self.skip_separators();
                    while !matches!(self.peek_kind(), TokenKind::RBrace) {
                        let (fname, fspan) = self.expect_ident("field name")?;
                        let (pat, fend) = if self.eat(&TokenKind::Colon) {
                            self.skip_newlines();
                            let p = self.parse_pattern()?;
                            let span_end = p.span.clone();
                            (Some(p), span_end)
                        } else {
                            (None, fspan.clone())
                        };
                        let fspan_full = merge_spans(&fspan, &fend);
                        fields.push(FieldPattern {
                            name: fname,
                            pattern: pat,
                            span: fspan_full,
                        });
                        // Field separator: `,` or newline or both.
                        if !self.eat(&TokenKind::Comma) {
                            // Newline alone is also a separator.
                            if !matches!(self.peek_kind(), TokenKind::Newline) {
                                break;
                            }
                        }
                        self.skip_separators();
                    }
                    let rbrace = self.expect(&TokenKind::RBrace, "`}`")?;
                    let span = merge_spans(&name_span, &rbrace.span);
                    return Ok(Pattern {
                        kind: PatternKind::Struct { name, fields },
                        span,
                    });
                }
                Ok(Pattern {
                    kind: PatternKind::Ident(name),
                    span: name_span,
                })
            }
            other => Err(crate::error::RavenError::parse(
                ParseError::InvalidPattern(format!("unexpected token in pattern: {:?}", other)),
                start,
            )),
        }
    }
}
