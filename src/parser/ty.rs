//! Type expression parsing.

use crate::ast::{Type, TypeKind, TypePath, TypePathSegment};
use crate::lexer::TokenKind;
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
                let mut items = Vec::new();
                while !matches!(self.peek_kind(), TokenKind::RParen) {
                    items.push(self.parse_type()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                let rparen = self.expect(&TokenKind::RParen, "`)`")?;
                let span = merge_spans(&start, &rparen.span);
                match items.len() {
                    0 => Ok(Type {
                        kind: TypeKind::Unit,
                        span,
                    }),
                    1 => Ok(items.remove(0)),
                    n if n <= 6 => Ok(Type {
                        kind: TypeKind::Path(crate::ast::TypePath {
                            segments: vec![crate::ast::TypePathSegment {
                                name: format!("Tuple{n}"),
                                generics: items,
                                span: span.clone(),
                            }],
                            span: span.clone(),
                        }),
                        span,
                    }),
                    _ => Err(crate::error::RavenError::parse(
                        crate::error::ParseError::Custom(
                            "a tuple holds at most 6 items; use a class".into(),
                        ),
                        span,
                    )),
                }
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
        if matches!(self.peek_kind(), TokenKind::LParen) {
            let ty = self.parse_type()?;
            return match ty.kind {
                TypeKind::Path(p) => Ok(p),
                _ => Err(crate::error::RavenError::parse(
                    crate::error::ParseError::Custom("expected a type name or tuple type".into()),
                    ty.span,
                )),
            };
        }
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
                let new = match n.as_str() {
                    "Int" => Some("int"),
                    "Float" => Some("float"),
                    "Bool" => Some("bool"),
                    "String" => Some("str"),
                    "Char" => Some("char"),
                    "List" => Some("list"),
                    "Map" => Some("dict"),
                    "Set" => Some("set"),
                    _ => None,
                };
                if let Some(new) = new {
                    return Err(self.legacy(&format!("the type `{n}`"), &format!("write `{new}`")));
                }
                self.advance();
                (canonical_type_name(&n).unwrap_or(&n).to_string(), tok.span)
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
            return Err(self.legacy("`Name<T>` type arguments", "write `Name[T]`"));
        } else if matches!(self.peek_kind(), TokenKind::LBracket) {
            let (args, args_span) = self.parse_bracket_type_args()?;
            generics = args;
            span = merge_spans(&span, &args_span);
        }
        Ok(TypePathSegment {
            name,
            generics,
            span,
        })
    }

    /// Parse `[T, U]`; the caller has seen the `[`.
    pub(crate) fn parse_bracket_type_args(&mut self) -> ParseResult<(Vec<Type>, Span)> {
        let start = self.advance().span;
        let mut args = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek_kind(), TokenKind::RBracket) {
            args.push(self.parse_type()?);
            self.skip_newlines();
            if !self.eat(&TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
        }
        let close = self.expect(&TokenKind::RBracket, "`]`")?;
        Ok((args, merge_spans(&start, &close.span)))
    }

    /// Whether `[` here opens type arguments rather than an index: the first
    /// bracketed token names a type (PascalCase or a builtin type name).
    pub(crate) fn bracket_opens_type_args(&self) -> bool {
        if !matches!(self.peek_kind(), TokenKind::LBracket) {
            return false;
        }
        match self.peek_kind_at(1) {
            TokenKind::Identifier(n) => {
                let builtin = matches!(
                    n.as_str(),
                    "int" | "float" | "bool" | "str" | "char" | "list" | "dict" | "set"
                );
                let pascal = n.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                    && (n.chars().any(|c| c.is_ascii_lowercase()) || n.len() == 1);
                builtin || pascal
            }
            TokenKind::SelfUpper => true,
            _ => false,
        }
    }
}

/// The canonical name of a lowercase builtin type spelling.
pub(crate) fn canonical_type_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "int" => "Int",
        "float" => "Float",
        "bool" => "Bool",
        "str" => "String",
        "char" => "Char",
        "list" => "List",
        "dict" => "Map",
        "set" => "Set",
        _ => return None,
    })
}
