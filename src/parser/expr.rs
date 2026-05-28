//! Expression parsing.
//!
//! Operator precedence is encoded with one recursive function per
//! level. The level order from lowest to highest matches the spec.

use crate::ast::{
    BinaryOp, Block, ElseBranch, Expr, ExprKind, FieldInit, LambdaBody, LambdaParam, MatchArm,
    StrFragment, Type, UnaryOp,
};
use crate::error::{ParseError, RavenError};
use crate::lexer::{Lexer, TokenKind, ESCAPED_DOLLAR_SENTINEL};
use crate::span::Span;

use super::{merge_spans, ParseResult, Parser};

impl Parser {
    /// Parse a full expression at the lowest precedence.
    pub(crate) fn parse_expr(&mut self) -> ParseResult<Expr> {
        self.parse_logical_or()
    }

    /// Parse an expression with struct literals temporarily disabled.
    /// Used for the condition / scrutinee of `if`, `while`, `for`, and
    /// `match` so a trailing `{` is the body, not a struct literal.
    pub(crate) fn parse_expr_no_struct(&mut self) -> ParseResult<Expr> {
        let saved = self.no_struct_literal;
        self.no_struct_literal = true;
        let r = self.parse_expr();
        self.no_struct_literal = saved;
        r
    }

    // ----- precedence ladder -----

    fn parse_logical_or(&mut self) -> ParseResult<Expr> {
        let mut lhs = self.parse_logical_and()?;
        loop {
            self.skip_newlines_at_continuation();
            if matches!(self.peek_kind(), TokenKind::OrOr) {
                self.advance();
                self.skip_newlines();
                let rhs = self.parse_logical_and()?;
                let span = merge_spans(&lhs.span, &rhs.span);
                lhs = Expr {
                    kind: ExprKind::Binary {
                        op: BinaryOp::Or,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    span,
                };
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_logical_and(&mut self) -> ParseResult<Expr> {
        let mut lhs = self.parse_comparison()?;
        loop {
            self.skip_newlines_at_continuation();
            if matches!(self.peek_kind(), TokenKind::AndAnd) {
                self.advance();
                self.skip_newlines();
                let rhs = self.parse_comparison()?;
                let span = merge_spans(&lhs.span, &rhs.span);
                lhs = Expr {
                    kind: ExprKind::Binary {
                        op: BinaryOp::And,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    span,
                };
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_comparison(&mut self) -> ParseResult<Expr> {
        let lhs = self.parse_bit_or()?;
        self.skip_newlines_at_continuation();
        let op = match self.peek_kind() {
            TokenKind::EqEq => Some(BinaryOp::Eq),
            TokenKind::NotEq => Some(BinaryOp::Ne),
            TokenKind::Lt => Some(BinaryOp::Lt),
            TokenKind::Gt => Some(BinaryOp::Gt),
            TokenKind::LtEq => Some(BinaryOp::Le),
            TokenKind::GtEq => Some(BinaryOp::Ge),
            _ => None,
        };
        let Some(op) = op else { return Ok(lhs) };
        self.advance();
        self.skip_newlines();
        let rhs = self.parse_bit_or()?;
        // Chain rejection: another comparison directly after rhs is a
        // parse error per the spec.
        self.skip_newlines_at_continuation();
        if matches!(
            self.peek_kind(),
            TokenKind::EqEq
                | TokenKind::NotEq
                | TokenKind::Lt
                | TokenKind::Gt
                | TokenKind::LtEq
                | TokenKind::GtEq
        ) {
            return Err(RavenError::parse(
                ParseError::ChainedComparison,
                self.peek().span.clone(),
            ));
        }
        let span = merge_spans(&lhs.span, &rhs.span);
        Ok(Expr {
            kind: ExprKind::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            span,
        })
    }

    fn parse_bit_or(&mut self) -> ParseResult<Expr> {
        self.parse_left_assoc(Self::parse_bit_xor, &[(TokenKind::Pipe, BinaryOp::BitOr)])
    }

    fn parse_bit_xor(&mut self) -> ParseResult<Expr> {
        self.parse_left_assoc(Self::parse_bit_and, &[(TokenKind::Caret, BinaryOp::BitXor)])
    }

    fn parse_bit_and(&mut self) -> ParseResult<Expr> {
        self.parse_left_assoc(Self::parse_shift, &[(TokenKind::Amp, BinaryOp::BitAnd)])
    }

    fn parse_shift(&mut self) -> ParseResult<Expr> {
        self.parse_left_assoc(
            Self::parse_range,
            &[
                (TokenKind::Shl, BinaryOp::Shl),
                (TokenKind::Shr, BinaryOp::Shr),
            ],
        )
    }

    fn parse_range(&mut self) -> ParseResult<Expr> {
        let lhs = self.parse_additive()?;
        self.skip_newlines_at_continuation();
        let inclusive = match self.peek_kind() {
            TokenKind::DotDot => false,
            TokenKind::DotDotEq => true,
            _ => return Ok(lhs),
        };
        self.advance();
        self.skip_newlines();
        let rhs = self.parse_additive()?;
        let span = merge_spans(&lhs.span, &rhs.span);
        Ok(Expr {
            kind: ExprKind::Range {
                start: Box::new(lhs),
                end: Box::new(rhs),
                inclusive,
            },
            span,
        })
    }

    fn parse_additive(&mut self) -> ParseResult<Expr> {
        self.parse_left_assoc(
            Self::parse_multiplicative,
            &[
                (TokenKind::Plus, BinaryOp::Add),
                (TokenKind::Minus, BinaryOp::Sub),
            ],
        )
    }

    fn parse_multiplicative(&mut self) -> ParseResult<Expr> {
        self.parse_left_assoc(
            Self::parse_unary,
            &[
                (TokenKind::Star, BinaryOp::Mul),
                (TokenKind::Slash, BinaryOp::Div),
                (TokenKind::Percent, BinaryOp::Mod),
            ],
        )
    }

    fn parse_left_assoc(
        &mut self,
        next: fn(&mut Parser) -> ParseResult<Expr>,
        ops: &[(TokenKind, BinaryOp)],
    ) -> ParseResult<Expr> {
        let mut lhs = next(self)?;
        loop {
            self.skip_newlines_at_continuation();
            let cur = self.peek_kind().clone();
            let matched = ops
                .iter()
                .find(|(tk, _)| std::mem::discriminant(tk) == std::mem::discriminant(&cur))
                .map(|(_, op)| *op);
            let Some(op) = matched else { break };
            self.advance();
            self.skip_newlines();
            let rhs = next(self)?;
            let span = merge_spans(&lhs.span, &rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> ParseResult<Expr> {
        let start = self.peek().span.clone();
        let op = match self.peek_kind() {
            TokenKind::Minus => Some(UnaryOp::Neg),
            TokenKind::Bang => Some(UnaryOp::Not),
            TokenKind::Amp => Some(UnaryOp::Ref),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let operand = self.parse_unary()?;
            let span = merge_spans(&start, &operand.span);
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op,
                    operand: Box::new(operand),
                },
                span,
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek_kind() {
                TokenKind::Dot => {
                    self.advance();
                    self.skip_newlines();
                    let (name, name_span) = self.expect_ident("field or method name")?;
                    // Optional explicit generics: `.name<T>`.
                    let mut generics = Vec::new();
                    if matches!(self.peek_kind(), TokenKind::Lt) {
                        if let Some(args) = self.try_parse_type_args_for_call() {
                            generics = args;
                        }
                    }
                    // Call form: `.name(args)`.
                    if matches!(self.peek_kind(), TokenKind::LParen) {
                        self.advance();
                        let (args, end_span) = self.parse_arg_list()?;
                        let span = merge_spans(&expr.span, &end_span);
                        expr = Expr {
                            kind: ExprKind::MethodCall {
                                receiver: Box::new(expr),
                                name,
                                generics,
                                args,
                            },
                            span,
                        };
                    } else {
                        let span = merge_spans(&expr.span, &name_span);
                        expr = Expr {
                            kind: ExprKind::Field {
                                receiver: Box::new(expr),
                                name,
                            },
                            span,
                        };
                    }
                }
                TokenKind::LParen => {
                    self.advance();
                    let (args, end_span) = self.parse_arg_list()?;
                    let span = merge_spans(&expr.span, &end_span);
                    expr = Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span,
                    };
                }
                TokenKind::LBracket => {
                    self.advance();
                    self.skip_newlines();
                    let index = self.parse_expr()?;
                    self.skip_newlines();
                    let rb = self.expect(&TokenKind::RBracket, "`]`")?;
                    let span = merge_spans(&expr.span, &rb.span);
                    expr = Expr {
                        kind: ExprKind::Index {
                            receiver: Box::new(expr),
                            index: Box::new(index),
                        },
                        span,
                    };
                }
                TokenKind::Question => {
                    let q = self.advance();
                    let span = merge_spans(&expr.span, &q.span);
                    expr = Expr {
                        kind: ExprKind::Try(Box::new(expr)),
                        span,
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    /// Parse the contents of an argument list (after `(`) and consume
    /// the closing `)`. Returns the args and the span of the closer.
    fn parse_arg_list(&mut self) -> ParseResult<(Vec<Expr>, Span)> {
        let mut args = Vec::new();
        self.skip_newlines();
        if !matches!(self.peek_kind(), TokenKind::RParen) {
            loop {
                self.skip_newlines();
                args.push(self.parse_expr()?);
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
        Ok((args, rparen.span))
    }

    fn parse_primary(&mut self) -> ParseResult<Expr> {
        let tok = self.peek().clone();
        match tok.kind.clone() {
            TokenKind::IntLit(n) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Int(n),
                    span: tok.span,
                })
            }
            TokenKind::FloatLit(v) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Float(v),
                    span: tok.span,
                })
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Bool(true),
                    span: tok.span,
                })
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Bool(false),
                    span: tok.span,
                })
            }
            TokenKind::StringLit(s) => {
                self.advance();
                self.string_literal_expr(s, tok.span)
            }
            TokenKind::BlockStringLit(s) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::BlockStr(s),
                    span: tok.span,
                })
            }
            TokenKind::CharLit(c) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Char(c),
                    span: tok.span,
                })
            }
            TokenKind::CStringLit(s) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::CStr(s),
                    span: tok.span,
                })
            }
            TokenKind::SelfLower => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::SelfLower,
                    span: tok.span,
                })
            }
            TokenKind::SelfUpper => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::SelfUpper,
                    span: tok.span,
                })
            }
            TokenKind::LParen => self.parse_paren_or_tuple(),
            TokenKind::LBracket => self.parse_array_lit(),
            TokenKind::LBrace => self.parse_brace_primary(),
            TokenKind::If => self.parse_if(),
            TokenKind::Match => self.parse_match(),
            TokenKind::Loop => self.parse_loop(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Fun => self.parse_lambda_fun(),
            TokenKind::Identifier(_) => self.parse_ident_primary(),
            _ => Err(self.unexpected("expression")),
        }
    }

    /// Build the expression for a `"..."` string literal token. The
    /// lexer has already decoded escapes and kept any real `${...}`
    /// interpolation verbatim, marking each escaped `\$` with the
    /// `ESCAPED_DOLLAR_SENTINEL`. This splits the decoded text into
    /// fragments. A literal with no real `${...}` becomes a plain
    /// `ExprKind::Str` (with any escaped dollars un-escaped). One or more
    /// real interpolations produce an `ExprKind::InterpolatedString`.
    fn string_literal_expr(&self, decoded: String, span: Span) -> ParseResult<Expr> {
        let fragments = split_interpolation(&decoded, &span)?;
        // Collapse to a plain string when no embedded expression survived.
        if fragments
            .iter()
            .all(|f| matches!(f, StrFragment::Literal(_)))
        {
            let mut buf = String::new();
            for f in &fragments {
                if let StrFragment::Literal(s) = f {
                    buf.push_str(s);
                }
            }
            return Ok(Expr {
                kind: ExprKind::Str(buf),
                span,
            });
        }
        Ok(Expr {
            kind: ExprKind::InterpolatedString(fragments),
            span,
        })
    }

    fn parse_paren_or_tuple(&mut self) -> ParseResult<Expr> {
        let lparen = self.advance();
        self.skip_newlines();
        // `()` is a unit value: we represent it as an empty tuple but
        // reject it as UnsupportedTuple for now. Actually `()` does not
        // appear in the spec as a value form, so reject.
        if matches!(self.peek_kind(), TokenKind::RParen) {
            return Err(RavenError::parse(ParseError::UnsupportedTuple, lparen.span));
        }
        let first = self.parse_expr()?;
        self.skip_newlines();
        if matches!(self.peek_kind(), TokenKind::RParen) {
            let rp = self.advance();
            let span = merge_spans(&lparen.span, &rp.span);
            return Ok(Expr {
                kind: ExprKind::Paren(Box::new(first)),
                span,
            });
        }
        // Tuple: at least one comma after `first`. Parse the rest of
        // the elements to give a useful span, then report
        // UnsupportedTuple.
        let _ = first;
        while self.eat(&TokenKind::Comma) {
            self.skip_newlines();
            if matches!(self.peek_kind(), TokenKind::RParen) {
                break;
            }
            self.parse_expr()?;
            self.skip_newlines();
        }
        let rp = self.expect(&TokenKind::RParen, "`)`")?;
        let span = merge_spans(&lparen.span, &rp.span);
        Err(RavenError::parse(ParseError::UnsupportedTuple, span))
    }

    fn parse_array_lit(&mut self) -> ParseResult<Expr> {
        let lb = self.advance();
        let mut items = Vec::new();
        self.skip_newlines();
        if !matches!(self.peek_kind(), TokenKind::RBracket) {
            loop {
                self.skip_newlines();
                items.push(self.parse_expr()?);
                self.skip_newlines();
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
                if matches!(self.peek_kind(), TokenKind::RBracket) {
                    break;
                }
            }
        }
        let rb = self.expect(&TokenKind::RBracket, "`]`")?;
        let span = merge_spans(&lb.span, &rb.span);
        Ok(Expr {
            kind: ExprKind::Array(items),
            span,
        })
    }

    /// Parse a `{...}` primary: either a shorthand lambda or a block
    /// expression. The distinguisher is an `Arrow` token at depth 0
    /// before any statement separator outside parens or brackets.
    fn parse_brace_primary(&mut self) -> ParseResult<Expr> {
        if self.is_shorthand_lambda() {
            return self.parse_lambda_shorthand();
        }
        let block = self.parse_block()?;
        let span = block.span.clone();
        Ok(Expr {
            kind: ExprKind::Block(block),
            span,
        })
    }

    /// Heuristic: look forward from the `{` to find an `Arrow` at brace
    /// depth 0 before we leave the brace. If found, the brace is a
    /// shorthand lambda.
    fn is_shorthand_lambda(&self) -> bool {
        // Cursor is at `{`. Scan forward.
        let mut depth_paren = 0i32;
        let mut depth_bracket = 0i32;
        let mut depth_brace = 0i32;
        let mut i = self.pos;
        while i < self.tokens.len() {
            let k = &self.tokens[i].kind;
            match k {
                TokenKind::LBrace => depth_brace += 1,
                TokenKind::RBrace => {
                    depth_brace -= 1;
                    if depth_brace == 0 {
                        return false;
                    }
                }
                TokenKind::LParen => depth_paren += 1,
                TokenKind::RParen => depth_paren -= 1,
                TokenKind::LBracket => depth_bracket += 1,
                TokenKind::RBracket => depth_bracket -= 1,
                TokenKind::Arrow if depth_brace == 1 && depth_paren == 0 && depth_bracket == 0 => {
                    return true;
                }
                TokenKind::Newline | TokenKind::Semi
                    if depth_brace == 1 && depth_paren == 0 && depth_bracket == 0 =>
                {
                    return false;
                }
                TokenKind::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn parse_lambda_shorthand(&mut self) -> ParseResult<Expr> {
        let lb = self.advance(); // {
        let mut params = Vec::new();
        // Optional parameter list followed by `->`.
        // It is guaranteed by is_shorthand_lambda that an Arrow exists
        // before the closing brace, but the parameter list may be
        // empty in degenerate cases. We require at least one ident if
        // followed by something other than `->`.
        loop {
            self.skip_newlines();
            if matches!(self.peek_kind(), TokenKind::Arrow) {
                break;
            }
            let (name, span) = self.expect_ident("lambda parameter name")?;
            params.push(LambdaParam {
                name,
                ty: None,
                span,
            });
            self.skip_newlines();
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.skip_newlines();
        self.expect(&TokenKind::Arrow, "`->`")?;
        self.skip_separators();
        // The body is the rest of the brace contents parsed as a block.
        let body_block = self.parse_block_body_until_rbrace()?;
        let rb = self.expect(&TokenKind::RBrace, "`}`")?;
        let span = merge_spans(&lb.span, &rb.span);
        let body_span = merge_spans(&lb.span, &rb.span);
        Ok(Expr {
            kind: ExprKind::Lambda {
                params,
                ret: None,
                body: LambdaBody::Block(Block {
                    span: body_span,
                    stmts: body_block.0,
                    trailing: body_block.1,
                }),
                params_inferred: true,
            },
            span,
        })
    }

    fn parse_lambda_fun(&mut self) -> ParseResult<Expr> {
        // `fun ( params ) [-> Type] body`
        let fun_tok = self.advance();
        self.expect(&TokenKind::LParen, "`(`")?;
        let mut params = Vec::new();
        if !matches!(self.peek_kind(), TokenKind::RParen) {
            loop {
                self.skip_newlines();
                let (name, name_span) = self.expect_ident("parameter name")?;
                self.expect(&TokenKind::Colon, "`:`")?;
                let ty = self.parse_type()?;
                let span = merge_spans(&name_span, &ty.span);
                params.push(LambdaParam {
                    name,
                    ty: Some(ty),
                    span,
                });
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
        let ret = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        // Body: `= expr` or block.
        let (body, body_span) = if self.eat(&TokenKind::Eq) {
            self.skip_newlines();
            let e = self.parse_expr()?;
            let s = e.span.clone();
            (LambdaBody::Expr(Box::new(e)), s)
        } else {
            let block = self.parse_block()?;
            let s = block.span.clone();
            (LambdaBody::Block(block), s)
        };
        let span = merge_spans(&fun_tok.span, &body_span);
        Ok(Expr {
            kind: ExprKind::Lambda {
                params,
                ret,
                body,
                params_inferred: false,
            },
            span,
        })
    }

    fn parse_ident_primary(&mut self) -> ParseResult<Expr> {
        let (name, name_span) = self.expect_ident("identifier")?;
        // Optional generics, only when the trial succeeds.
        let mut generics = Vec::new();
        if matches!(self.peek_kind(), TokenKind::Lt) {
            if let Some(args) = self.try_parse_type_args_for_call() {
                generics = args;
            }
        }
        // Struct literal: `Name { ... }` when struct literals are not
        // suppressed in the current context.
        if !self.no_struct_literal && matches!(self.peek_kind(), TokenKind::LBrace) {
            // Look further: it could still be a block following another
            // expression in some weird context. In the spec, a `{` right
            // after an ident in expression position is a struct literal,
            // and the `no_struct_literal` flag covers the if/while/for
            // exception.
            let saved = self.checkpoint();
            self.advance(); // {
                            // Probe: a struct literal field list is either empty,
                            // shorthand idents, or `name: expr` pairs. If the first
                            // non separator token is not an identifier followed by
                            // `:`, `,`, newline, or `}`, treat as not a struct lit.
            self.skip_separators();
            let probe_ok = match self.peek_kind() {
                TokenKind::RBrace => true,
                TokenKind::Identifier(_) => matches!(
                    self.peek_kind_at(1),
                    TokenKind::Colon | TokenKind::Comma | TokenKind::Newline | TokenKind::RBrace
                ),
                _ => false,
            };
            if probe_ok {
                let mut fields: Vec<FieldInit> = Vec::new();
                while !matches!(self.peek_kind(), TokenKind::RBrace) {
                    self.skip_separators();
                    if matches!(self.peek_kind(), TokenKind::RBrace) {
                        break;
                    }
                    let (fname, fspan) = self.expect_ident("field name")?;
                    let (value, vspan) = if self.eat(&TokenKind::Colon) {
                        self.skip_newlines();
                        let v = self.parse_expr()?;
                        let s = v.span.clone();
                        (v, s)
                    } else {
                        // Shorthand: `x` means `x: x` with same span.
                        (
                            Expr {
                                kind: ExprKind::Ident {
                                    name: fname.clone(),
                                    generics: Vec::new(),
                                },
                                span: fspan.clone(),
                            },
                            fspan.clone(),
                        )
                    };
                    // Duplicate detection.
                    if fields.iter().any(|f| f.name == fname) {
                        return Err(RavenError::parse(ParseError::DuplicateField(fname), fspan));
                    }
                    let full_span = merge_spans(&fspan, &vspan);
                    fields.push(FieldInit {
                        name: fname,
                        value,
                        span: full_span,
                    });
                    if !self.eat(&TokenKind::Comma) {
                        // Newline alone is also accepted as a separator.
                        if !matches!(self.peek_kind(), TokenKind::Newline | TokenKind::RBrace) {
                            break;
                        }
                    }
                    self.skip_separators();
                }
                let rb = self.expect(&TokenKind::RBrace, "`}`")?;
                let span = merge_spans(&name_span, &rb.span);
                return Ok(Expr {
                    kind: ExprKind::StructLit {
                        name,
                        generics,
                        fields,
                    },
                    span,
                });
            }
            // Rewind: this `{` is not a struct literal after all.
            self.rewind(saved);
        }
        Ok(Expr {
            kind: ExprKind::Ident { name, generics },
            span: name_span,
        })
    }

    /// Try to parse a `< ... >` type argument list for a call or path
    /// in expression context. On failure (it was a comparison after
    /// all), rewinds and returns `None`.
    pub(crate) fn try_parse_type_args_for_call(&mut self) -> Option<Vec<Type>> {
        let saved = self.checkpoint();
        // Quick reject: if the token after `<` is one that can never
        // start a type, bail out.
        match self.peek_kind_at(1) {
            TokenKind::Identifier(_)
            | TokenKind::SelfUpper
            | TokenKind::Fun
            | TokenKind::LParen => {}
            _ => return None,
        }
        let try_args = self.parse_type_args_required();
        let Ok((args, _span)) = try_args else {
            self.rewind(saved);
            return None;
        };
        // Confirm: the token after the closing angle must be one of
        // the followers that indicate a generic application.
        match self.peek_kind() {
            TokenKind::LParen
            | TokenKind::Dot
            | TokenKind::ColonColon
            | TokenKind::LBrace
            | TokenKind::Eq
            | TokenKind::Comma
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::RBrace
            | TokenKind::Semi
            | TokenKind::Newline
            | TokenKind::Eof => Some(args),
            _ => {
                self.rewind(saved);
                None
            }
        }
    }

    // ----- control flow -----

    fn parse_if(&mut self) -> ParseResult<Expr> {
        let if_tok = self.advance();
        let cond = self.parse_expr_no_struct()?;
        let then_branch = self.parse_block()?;
        let mut span = merge_spans(&if_tok.span, &then_branch.span);
        let else_branch = if self.eat_else() {
            self.skip_newlines();
            if matches!(self.peek_kind(), TokenKind::If) {
                let nested = self.parse_if()?;
                span = merge_spans(&span, &nested.span);
                Some(Box::new(ElseBranch::If(nested)))
            } else {
                let block = self.parse_block()?;
                span = merge_spans(&span, &block.span);
                Some(Box::new(ElseBranch::Block(block)))
            }
        } else {
            None
        };
        Ok(Expr {
            kind: ExprKind::If {
                cond: Box::new(cond),
                then_branch,
                else_branch,
            },
            span,
        })
    }

    /// Consume an `else` even if separated from the previous block by
    /// newlines.
    fn eat_else(&mut self) -> bool {
        let saved = self.checkpoint();
        self.skip_newlines();
        if self.eat(&TokenKind::Else) {
            true
        } else {
            self.rewind(saved);
            false
        }
    }

    fn parse_match(&mut self) -> ParseResult<Expr> {
        let m = self.advance();
        let scrutinee = self.parse_expr_no_struct()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        self.skip_separators();
        let mut arms = Vec::new();
        while !matches!(self.peek_kind(), TokenKind::RBrace) {
            let pattern = self.parse_pattern()?;
            let guard = if self.eat(&TokenKind::If) {
                Some(self.parse_expr_no_struct()?)
            } else {
                None
            };
            self.expect(&TokenKind::Arrow, "`->`")?;
            self.skip_newlines();
            let body = self.parse_expr()?;
            let span_start = pattern.span.clone();
            let span_end = body.span.clone();
            let span = merge_spans(&span_start, &span_end);
            arms.push(MatchArm {
                pattern,
                guard,
                body,
                span,
            });
            // Arm separator: `,` or newline.
            if !self.eat(&TokenKind::Comma) && !matches!(self.peek_kind(), TokenKind::Newline) {
                break;
            }
            self.skip_separators();
        }
        let rb = self.expect(&TokenKind::RBrace, "`}`")?;
        let span = merge_spans(&m.span, &rb.span);
        Ok(Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            span,
        })
    }

    fn parse_loop(&mut self) -> ParseResult<Expr> {
        let lp = self.advance();
        let body = self.parse_block()?;
        let span = merge_spans(&lp.span, &body.span);
        Ok(Expr {
            kind: ExprKind::Loop(body),
            span,
        })
    }

    fn parse_while(&mut self) -> ParseResult<Expr> {
        let w = self.advance();
        let cond = self.parse_expr_no_struct()?;
        let body = self.parse_block()?;
        let span = merge_spans(&w.span, &body.span);
        Ok(Expr {
            kind: ExprKind::While {
                cond: Box::new(cond),
                body,
            },
            span,
        })
    }

    fn parse_for(&mut self) -> ParseResult<Expr> {
        let f = self.advance();
        let pattern = self.parse_pattern()?;
        self.expect(&TokenKind::In, "`in`")?;
        let iter = self.parse_expr_no_struct()?;
        let body = self.parse_block()?;
        let span = merge_spans(&f.span, &body.span);
        Ok(Expr {
            kind: ExprKind::For {
                pattern,
                iter: Box::new(iter),
                body,
            },
            span,
        })
    }

    // ----- blocks -----

    /// Parse a `{...}` block expression.
    pub(crate) fn parse_block(&mut self) -> ParseResult<Block> {
        let lb = self.expect(&TokenKind::LBrace, "`{`")?;
        self.skip_separators();
        let (stmts, trailing) = self.parse_block_body_until_rbrace()?;
        let rb = self.expect(&TokenKind::RBrace, "`}`")?;
        let span = merge_spans(&lb.span, &rb.span);
        Ok(Block {
            stmts,
            trailing,
            span,
        })
    }

    /// Parse the body of a block (everything between `{` and `}`) and
    /// return the statement vector plus optional trailing expression.
    /// The caller is responsible for consuming the surrounding braces.
    fn parse_block_body_until_rbrace(
        &mut self,
    ) -> ParseResult<(Vec<crate::ast::Stmt>, Option<Box<Expr>>)> {
        use crate::ast::{Stmt, StmtKind};
        let mut stmts: Vec<Stmt> = Vec::new();
        loop {
            self.skip_separators();
            if matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof) {
                break;
            }
            let stmt = self.parse_stmt()?;
            // Detect trailing expression: a bare expression statement
            // followed by only newlines (not semicolons) before the
            // closing `}` becomes the block's value. A `;` terminates
            // the expression as a statement explicitly.
            let is_expr_stmt = matches!(stmt.kind, StmtKind::Expr(_));
            let mut at_trailing = is_expr_stmt;
            if at_trailing {
                let saved = self.checkpoint();
                while matches!(self.peek_kind(), TokenKind::Newline) {
                    self.advance();
                }
                let reached_brace = matches!(self.peek_kind(), TokenKind::RBrace);
                if !reached_brace {
                    self.rewind(saved);
                    at_trailing = false;
                }
            }
            if at_trailing {
                let trailing = match stmt.kind {
                    StmtKind::Expr(e) => e,
                    _ => unreachable!(),
                };
                return Ok((stmts, Some(Box::new(trailing))));
            }
            stmts.push(stmt);
        }
        Ok((stmts, None))
    }

    // ----- newline helpers -----

    /// Consume newlines only when they sit between two expression
    /// tokens. Used at every continuation point in the precedence
    /// ladder, called after the LHS has been parsed.
    pub(crate) fn skip_newlines_at_continuation(&mut self) {
        // Look ahead past newlines: if what follows is a binary
        // operator or postfix opener, consume them. Otherwise leave
        // the newline in place (it may terminate the statement).
        let saved = self.checkpoint();
        self.skip_newlines();
        if self.is_continuation_token() {
            // Newlines have been consumed.
        } else {
            self.rewind(saved);
        }
    }

    fn is_continuation_token(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Percent
                | TokenKind::EqEq
                | TokenKind::NotEq
                | TokenKind::Lt
                | TokenKind::Gt
                | TokenKind::LtEq
                | TokenKind::GtEq
                | TokenKind::AndAnd
                | TokenKind::OrOr
                | TokenKind::Amp
                | TokenKind::Pipe
                | TokenKind::Caret
                | TokenKind::Shl
                | TokenKind::Shr
                | TokenKind::DotDot
                | TokenKind::DotDotEq
                | TokenKind::Dot
                | TokenKind::Question
                | TokenKind::LParen
                | TokenKind::LBracket
        )
    }
}

/// Split the decoded text of a `"..."` literal into interpolation
/// fragments. The lexer has already decoded escapes; a real `${...}`
/// appears verbatim as the bytes `$ { ... }`, while an escaped `\$`
/// appears as [`ESCAPED_DOLLAR_SENTINEL`] immediately followed by `$`.
///
/// Each real `${...}` snippet is re-lexed and re-parsed as a standalone
/// Raven expression. The embedded expression's spans are anchored to a
/// synthetic per-fragment source path so that the resolver, type
/// checker, and lowering passes (all of which key on file plus byte
/// range) never collide a fragment's spans with the surrounding source
/// or with another fragment.
///
/// A literal with no real `${...}` yields a single
/// [`StrFragment::Literal`] (with the sentinel stripped). The caller
/// collapses an all-literal result back to a plain `ExprKind::Str`.
fn split_interpolation(decoded: &str, span: &Span) -> ParseResult<Vec<StrFragment>> {
    let mut fragments: Vec<StrFragment> = Vec::new();
    let mut text = String::new();
    let mut frag_index = 0usize;
    let bytes = decoded.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // An escaped dollar is the sentinel followed by `$`; emit a
        // literal `$` and skip the sentinel so `\${x}` is literal text.
        if decoded[i..].starts_with(ESCAPED_DOLLAR_SENTINEL) {
            let sentinel_len = ESCAPED_DOLLAR_SENTINEL.len_utf8();
            text.push('$');
            // Skip the sentinel and the `$` byte that follows it.
            i += sentinel_len;
            if i < bytes.len() && bytes[i] == b'$' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Find the matching `}` with brace-depth tracking so a
            // nested `{ }` inside the expression does not close early.
            let start = i + 2;
            let mut depth = 1usize;
            let mut j = start;
            while j < bytes.len() {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if depth != 0 {
                return Err(RavenError::parse(
                    ParseError::Custom("unterminated `${` interpolation in string literal".into()),
                    span.clone(),
                ));
            }
            // Flush the pending literal text.
            if !text.is_empty() {
                fragments.push(StrFragment::Literal(std::mem::take(&mut text)));
            }
            let snippet = &decoded[start..j];
            if snippet.trim().is_empty() {
                return Err(RavenError::parse(
                    ParseError::Custom("empty `${}` interpolation in string literal".into()),
                    span.clone(),
                ));
            }
            let expr = parse_interpolation_snippet(snippet, span, frag_index)?;
            fragments.push(StrFragment::Expr(Box::new(expr)));
            frag_index += 1;
            i = j + 1;
            continue;
        }
        let c = decoded[i..].chars().next().unwrap_or(' ');
        text.push(c);
        i += c.len_utf8();
    }
    if !text.is_empty() {
        fragments.push(StrFragment::Literal(text));
    }
    Ok(fragments)
}

/// Re-lex and re-parse a single `${...}` snippet into an [`Expr`].
///
/// The snippet is parsed against a synthetic source file whose path is
/// derived from the enclosing literal's span and the fragment index, so
/// every fragment's spans occupy a private `(file, byte-range)` keyspace
/// that cannot collide with real source spans. Parse errors are
/// re-anchored to the literal's span so the diagnostic points the reader
/// at the offending string.
fn parse_interpolation_snippet(snippet: &str, span: &Span, frag_index: usize) -> ParseResult<Expr> {
    let synthetic = std::path::PathBuf::from(format!(
        "{}<interp:{}:{}>",
        span.file.display(),
        span.start,
        frag_index
    ));
    let tokens = Lexer::new(snippet.to_string(), synthetic)
        .tokenize()
        .map_err(|e| reanchor(e, span))?;
    let mut parser = Parser::new(&tokens);
    parser.skip_newlines();
    let expr = parser.parse_expr().map_err(|e| reanchor(e, span))?;
    parser.skip_newlines();
    if !parser.is_at_end() {
        return Err(RavenError::parse(
            ParseError::Custom(format!(
                "`${{{}}}` interpolation must contain a single expression",
                snippet.trim()
            )),
            span.clone(),
        ));
    }
    Ok(expr)
}

/// Re-anchor an error raised while parsing an interpolation snippet onto
/// the enclosing string literal's span, preserving the error kind and
/// hint. The snippet's synthetic span is not useful to a reader.
fn reanchor(err: RavenError, span: &Span) -> RavenError {
    match err {
        RavenError::Lex(k, _, h) => {
            let mut e = RavenError::Lex(k, span.clone(), None);
            if let Some(h) = h {
                e = e.with_hint(h);
            }
            e
        }
        RavenError::Parse(k, _, h) => {
            let mut e = RavenError::Parse(k, span.clone(), None);
            if let Some(h) = h {
                e = e.with_hint(h);
            }
            e
        }
        RavenError::Resolve(k, _, h) => {
            let mut e = RavenError::Resolve(k, span.clone(), None);
            if let Some(h) = h {
                e = e.with_hint(h);
            }
            e
        }
        RavenError::Type(k, _, h) => {
            let mut e = RavenError::Type(k, span.clone(), None);
            if let Some(h) = h {
                e = e.with_hint(h);
            }
            e
        }
    }
}
