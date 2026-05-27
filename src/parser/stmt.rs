//! Statement parsing inside block bodies.

use crate::ast::{AssignOp, Expr, ExprKind, Stmt, StmtKind};
use crate::error::{ParseError, RavenError};
use crate::lexer::TokenKind;

use super::{merge_spans, ParseResult, Parser};

impl Parser {
    /// Parse one statement.
    pub(crate) fn parse_stmt(&mut self) -> ParseResult<Stmt> {
        let start_span = self.peek().span.clone();
        match self.peek_kind() {
            TokenKind::Let => self.parse_let_stmt(),
            TokenKind::Return => {
                self.advance();
                if matches!(
                    self.peek_kind(),
                    TokenKind::Newline | TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof
                ) {
                    return Ok(Stmt {
                        kind: StmtKind::Return(None),
                        span: start_span,
                    });
                }
                let value = self.parse_expr()?;
                let span = merge_spans(&start_span, &value.span);
                Ok(Stmt {
                    kind: StmtKind::Return(Some(value)),
                    span,
                })
            }
            TokenKind::Break => {
                self.advance();
                if matches!(
                    self.peek_kind(),
                    TokenKind::Newline | TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof
                ) {
                    return Ok(Stmt {
                        kind: StmtKind::Break(None),
                        span: start_span,
                    });
                }
                let value = self.parse_expr()?;
                let span = merge_spans(&start_span, &value.span);
                Ok(Stmt {
                    kind: StmtKind::Break(Some(value)),
                    span,
                })
            }
            TokenKind::Continue => {
                let tok = self.advance();
                Ok(Stmt {
                    kind: StmtKind::Continue,
                    span: tok.span,
                })
            }
            TokenKind::Defer => {
                self.advance();
                let value = self.parse_expr()?;
                let span = merge_spans(&start_span, &value.span);
                Ok(Stmt {
                    kind: StmtKind::Defer(value),
                    span,
                })
            }
            _ => self.parse_expr_or_assign_stmt(),
        }
    }

    fn parse_let_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.advance().span;
        let (name, _name_span) = self.expect_ident("identifier")?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        // Inside a function, the initializer is required.
        if !matches!(self.peek_kind(), TokenKind::Eq) {
            return Err(RavenError::parse(
                ParseError::UnexpectedToken {
                    expected: "`=`".to_string(),
                    found: super::describe_token(self.peek_kind()),
                },
                self.peek().span.clone(),
            ));
        }
        self.advance(); // =
        self.skip_newlines();
        let init = self.parse_expr()?;
        let span = merge_spans(&start, &init.span);
        Ok(Stmt {
            kind: StmtKind::Let {
                name,
                ty,
                init: Some(init),
            },
            span,
        })
    }

    fn parse_expr_or_assign_stmt(&mut self) -> ParseResult<Stmt> {
        let expr = self.parse_expr()?;
        let op = match self.peek_kind() {
            TokenKind::Eq => Some(AssignOp::Assign),
            TokenKind::PlusEq => Some(AssignOp::Add),
            TokenKind::MinusEq => Some(AssignOp::Sub),
            TokenKind::StarEq => Some(AssignOp::Mul),
            TokenKind::SlashEq => Some(AssignOp::Div),
            TokenKind::PercentEq => Some(AssignOp::Mod),
            TokenKind::AmpEq => Some(AssignOp::BitAnd),
            TokenKind::PipeEq => Some(AssignOp::BitOr),
            TokenKind::CaretEq => Some(AssignOp::BitXor),
            TokenKind::ShlEq => Some(AssignOp::Shl),
            TokenKind::ShrEq => Some(AssignOp::Shr),
            _ => None,
        };
        let Some(op) = op else {
            let span = expr.span.clone();
            return Ok(Stmt {
                kind: StmtKind::Expr(expr),
                span,
            });
        };
        // Validate LValue shape.
        if !is_valid_lvalue(&expr) {
            return Err(RavenError::parse(
                ParseError::InvalidAssignmentTarget,
                expr.span,
            ));
        }
        self.advance();
        self.skip_newlines();
        let value = self.parse_expr()?;
        let span = merge_spans(&expr.span, &value.span);
        Ok(Stmt {
            kind: StmtKind::Assign {
                target: expr,
                op,
                value,
            },
            span,
        })
    }
}

/// True when `expr` is a syntactically valid assignment target:
/// identifier, field access, or index, possibly nested.
fn is_valid_lvalue(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Ident { .. } => true,
        ExprKind::Field { receiver, .. } => is_valid_lvalue(receiver),
        ExprKind::Index { receiver, .. } => is_valid_lvalue(receiver),
        ExprKind::Paren(inner) => is_valid_lvalue(inner),
        _ => false,
    }
}
