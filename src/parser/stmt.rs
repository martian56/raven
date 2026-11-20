//! Statement parsing inside block bodies.

use crate::ast::{AssignOp, Block, Expr, ExprKind, MatchArm, Pattern, PatternKind, Stmt, StmtKind};
use crate::error::{ParseError, RavenError};
use crate::lexer::TokenKind;

use super::{merge_spans, ParseResult, Parser};

impl Parser {
    /// Parse one statement.
    pub(crate) fn parse_stmt(&mut self) -> ParseResult<Stmt> {
        let start_span = self.peek().span.clone();
        match self.peek_kind() {
            TokenKind::Let => self.parse_let_stmt(false),
            TokenKind::Identifier(w)
                if w == "var"
                    && matches!(
                        self.peek_kind_at(1),
                        TokenKind::Identifier(_) | TokenKind::LParen
                    ) =>
            {
                self.parse_let_stmt(true)
            }
            TokenKind::Const => self.parse_let_stmt(false),
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
            TokenKind::LBrace => {
                let value = self.parse_block_or_expr()?;
                let span = value.span.clone();
                Ok(Stmt {
                    kind: StmtKind::Expr(value),
                    span,
                })
            }
            TokenKind::Defer => {
                self.advance();
                let value = self.parse_block_or_expr()?;
                let span = merge_spans(&start_span, &value.span);
                Ok(Stmt {
                    kind: StmtKind::Defer(value),
                    span,
                })
            }
            TokenKind::Spawn => {
                self.advance();
                let value = if matches!(self.peek_kind(), TokenKind::LBrace) {
                    let block = self.parse_block()?;
                    let span = block.span.clone();
                    Expr {
                        kind: ExprKind::Lambda {
                            params: Vec::new(),
                            ret: None,
                            body: crate::ast::LambdaBody::Block(block),
                            params_inferred: false,
                        },
                        span,
                    }
                } else {
                    self.parse_expr()?
                };
                let span = merge_spans(&start_span, &value.span);
                Ok(Stmt {
                    kind: StmtKind::Spawn(value),
                    span,
                })
            }
            _ => self.parse_expr_or_assign_stmt(),
        }
    }

    /// Parse a `let` (`mutable = true`) or `const` (`mutable = false`) local
    /// binding statement. Both require an initializer inside a function body.
    fn parse_let_stmt(&mut self, mutable: bool) -> ParseResult<Stmt> {
        if !matches!(self.peek_kind_at(2), TokenKind::Colon | TokenKind::Eq) {
            return self.parse_let_else(mutable);
        }
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
                mutable,
            },
            span,
        })
    }

    /// `let P = e else { .. }` as `let x = match e { P -> x, _ -> { .. } }`.
    fn parse_let_else(&mut self, mutable: bool) -> ParseResult<Stmt> {
        let start = self.advance().span;
        let pattern = self.parse_pattern()?;
        self.expect(&TokenKind::Eq, "`=`")?;
        self.skip_newlines();
        if matches!(pattern.kind, PatternKind::Tuple { name: None, .. }) {
            let init = self.parse_expr()?;
            if !matches!(self.peek_kind(), TokenKind::Else) {
                let span = merge_spans(&start, &init.span);
                return Ok(Stmt {
                    kind: StmtKind::LetTuple {
                        pattern,
                        init,
                        mutable,
                    },
                    span,
                });
            }
            return Err(RavenError::parse(
                ParseError::Custom("a tuple pattern always matches; drop the `else`".into()),
                self.peek().span.clone(),
            ));
        }
        let init = self.parse_expr_no_struct()?;
        if !self.eat(&TokenKind::Else) {
            return Err(RavenError::parse(
                ParseError::UnexpectedToken {
                    expected: "`else` after a pattern `let`".to_string(),
                    found: super::describe_token(self.peek_kind()),
                },
                self.peek().span.clone(),
            ));
        }
        let else_block = self.parse_block()?;
        let span = merge_spans(&start, &else_block.span);
        let mut names = Vec::new();
        pattern_bindings(&pattern, &mut names);
        let else_span = else_block.span.clone();
        let else_arm = MatchArm {
            pattern: Pattern {
                kind: PatternKind::Wildcard,
                span: start.clone(),
            },
            guard: None,
            span: else_span.clone(),
            body: Expr {
                kind: ExprKind::Block(else_block),
                span: else_span,
            },
        };
        match names.len() {
            0 => {
                let unit = Expr {
                    kind: ExprKind::Block(Block {
                        stmts: Vec::new(),
                        trailing: None,
                        span: pattern.span.clone(),
                    }),
                    span: pattern.span.clone(),
                };
                let m = Expr {
                    kind: ExprKind::Match {
                        scrutinee: Box::new(init),
                        arms: vec![
                            MatchArm {
                                span: pattern.span.clone(),
                                pattern,
                                guard: None,
                                body: unit,
                            },
                            else_arm,
                        ],
                    },
                    span: span.clone(),
                };
                Ok(Stmt {
                    kind: StmtKind::Expr(m),
                    span,
                })
            }
            1 => {
                let name = names.remove(0);
                let value = Expr {
                    kind: ExprKind::Ident {
                        name: name.clone(),
                        generics: Vec::new(),
                    },
                    span: start.clone(),
                };
                let m = Expr {
                    kind: ExprKind::Match {
                        scrutinee: Box::new(init),
                        arms: vec![
                            MatchArm {
                                span: pattern.span.clone(),
                                pattern,
                                guard: None,
                                body: value,
                            },
                            else_arm,
                        ],
                    },
                    span: span.clone(),
                };
                Ok(Stmt {
                    kind: StmtKind::Let {
                        name,
                        ty: None,
                        init: Some(m),
                        mutable,
                    },
                    span,
                })
            }
            _ => Err(RavenError::parse(
                ParseError::Custom(
                    "a pattern `let ... else` can bind one name; match the value instead".into(),
                ),
                pattern.span.clone(),
            )),
        }
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

/// True when `expr` is a syntactically valid assignment target.
///
/// A target is a place rooted in a name: a plain identifier or `self`,
/// optionally followed by any chain of field accesses (`a.b.c`) and
/// indexes (`xs[i]`, `obj.items[k]`). Literals, calls, and other value
/// producing expressions are rejected. The receiver of a field or index
/// in a target position is itself a target, so nesting is checked
/// recursively.
fn is_valid_lvalue(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Ident { .. } | ExprKind::SelfLower => true,
        ExprKind::Field { receiver, .. } => is_valid_lvalue(receiver),
        ExprKind::Index { receiver, .. } => is_valid_lvalue(receiver),
        ExprKind::Paren(inner) => is_valid_lvalue(inner),
        _ => false,
    }
}

fn pattern_bindings(p: &Pattern, out: &mut Vec<String>) {
    match &p.kind {
        PatternKind::Ident(n) => {
            if !n.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                out.push(n.clone());
            }
        }
        PatternKind::Tuple { elements, .. } => {
            for e in elements {
                pattern_bindings(e, out);
            }
        }
        PatternKind::Struct { fields, .. } => {
            for f in fields {
                match &f.pattern {
                    Some(sub) => pattern_bindings(sub, out),
                    None => out.push(f.name.clone()),
                }
            }
        }
        _ => {}
    }
}
