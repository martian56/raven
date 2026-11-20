//! Mutable post-order traversal over the expressions and blocks of a
//! declaration.

use super::decl::{Decl, DeclKind, Function, FunctionBody};
use super::expr::{Block, ElseBranch, Expr, ExprKind, LambdaBody, StrFragment};
use super::stmt::StmtKind;

/// Call `f` on every expression in `decl`, children before parents.
pub fn exprs_in_decl(decl: &mut Decl, f: &mut dyn FnMut(&mut Expr)) {
    walk_decl(decl, f, &mut |_| {});
}

/// Call `on_expr` on every expression and `on_block` on every block, children
/// before parents.
pub fn walk_decl(
    decl: &mut Decl,
    on_expr: &mut dyn FnMut(&mut Expr),
    on_block: &mut dyn FnMut(&mut Block),
) {
    let mut w = Walker { on_expr, on_block };
    match &mut decl.kind {
        DeclKind::Function(func) => w.function(func),
        DeclKind::Impl(i) => {
            for func in &mut i.items {
                w.function(func);
            }
        }
        DeclKind::Trait(t) => {
            for func in &mut t.members {
                w.function(func);
            }
        }
        DeclKind::Struct(s) => {
            for func in &mut s.methods {
                w.function(func);
            }
            for field in &mut s.fields {
                if let Some(d) = &mut field.default {
                    w.expr(d);
                }
            }
        }
        DeclKind::Const(c) => w.expr(&mut c.value),
        DeclKind::Let(l) => {
            if let Some(e) = &mut l.init {
                w.expr(e);
            }
        }
        _ => {}
    }
}

pub fn exprs_in_fn(func: &mut Function, f: &mut dyn FnMut(&mut Expr)) {
    let mut w = Walker {
        on_expr: f,
        on_block: &mut |_| {},
    };
    w.function(func);
}

pub fn expr(e: &mut Expr, f: &mut dyn FnMut(&mut Expr)) {
    let mut w = Walker {
        on_expr: f,
        on_block: &mut |_| {},
    };
    w.expr(e);
}

struct Walker<'a> {
    on_expr: &'a mut dyn FnMut(&mut Expr),
    on_block: &'a mut dyn FnMut(&mut Block),
}

impl Walker<'_> {
    fn function(&mut self, func: &mut Function) {
        for p in &mut func.params {
            if let Some(d) = &mut p.default {
                self.expr(d);
            }
        }
        match &mut func.body {
            FunctionBody::Block(b) => self.block(b),
            FunctionBody::Expr(e) => self.expr(e),
            FunctionBody::None => {}
        }
    }

    fn block(&mut self, b: &mut Block) {
        for s in &mut b.stmts {
            match &mut s.kind {
                StmtKind::Let { init, .. } => {
                    if let Some(e) = init {
                        self.expr(e);
                    }
                }
                StmtKind::LetElse {
                    init, else_block, ..
                } => {
                    self.expr(init);
                    self.block(else_block);
                }
                StmtKind::LetTuple { init, .. } => self.expr(init),
                StmtKind::Assert { cond, message, .. } => {
                    self.expr(cond);
                    if let Some(m) = message {
                        self.expr(m);
                    }
                }
                StmtKind::Return(e) | StmtKind::Break(e) => {
                    if let Some(e) = e {
                        self.expr(e);
                    }
                }
                StmtKind::Continue => {}
                StmtKind::Defer(e) | StmtKind::Spawn(e) | StmtKind::Expr(e) => self.expr(e),
                StmtKind::Assign { target, value, .. } => {
                    self.expr(target);
                    self.expr(value);
                }
            }
        }
        if let Some(t) = &mut b.trailing {
            self.expr(t);
        }
        (self.on_block)(b);
    }

    fn else_branch(&mut self, eb: &mut ElseBranch) {
        match eb {
            ElseBranch::If(x) => self.expr(x),
            ElseBranch::Block(b) => self.block(b),
        }
    }

    fn expr(&mut self, e: &mut Expr) {
        match &mut e.kind {
            ExprKind::InterpolatedString(frags) => {
                for frag in frags {
                    if let StrFragment::Expr(x) = frag {
                        self.expr(x);
                    }
                }
            }
            ExprKind::StructLit { fields, .. } => {
                for fi in fields {
                    self.expr(&mut fi.value);
                }
            }
            ExprKind::Array(xs) | ExprKind::SetLit(xs) | ExprKind::Tuple(xs) => {
                for x in xs {
                    self.expr(x);
                }
            }
            ExprKind::MapLit(pairs) => {
                for (k, v) in pairs {
                    self.expr(k);
                    self.expr(v);
                }
            }
            ExprKind::Comprehension {
                key,
                value,
                iter,
                cond,
                ..
            } => {
                if let Some(k) = key {
                    self.expr(k);
                }
                self.expr(value);
                self.expr(iter);
                if let Some(c) = cond {
                    self.expr(c);
                }
            }
            ExprKind::Paren(x)
            | ExprKind::Try(x)
            | ExprKind::NamedArg { value: x, .. }
            | ExprKind::Cast { value: x, .. } => self.expr(x),
            ExprKind::Block(b) | ExprKind::Loop(b) => self.block(b),
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Range { start, end, .. } => {
                self.expr(start);
                self.expr(end);
            }
            ExprKind::Call { callee, args } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                self.expr(receiver);
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::Field { receiver, .. } => self.expr(receiver),
            ExprKind::Index { receiver, index } => {
                self.expr(receiver);
                self.expr(index);
            }
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.expr(cond);
                self.block(then_branch);
                if let Some(eb) = else_branch {
                    self.else_branch(eb);
                }
            }
            ExprKind::IfLet {
                scrutinee,
                then_branch,
                else_branch,
                ..
            } => {
                self.expr(scrutinee);
                self.block(then_branch);
                if let Some(eb) = else_branch {
                    self.else_branch(eb);
                }
            }
            ExprKind::WhileLet {
                scrutinee, body, ..
            } => {
                self.expr(scrutinee);
                self.block(body);
            }
            ExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for a in arms {
                    if let Some(g) = &mut a.guard {
                        self.expr(g);
                    }
                    self.expr(&mut a.body);
                }
            }
            ExprKind::While { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            ExprKind::For { iter, body, .. } => {
                self.expr(iter);
                self.block(body);
            }
            ExprKind::Lambda { body, .. } => match body {
                LambdaBody::Block(b) => self.block(b),
                LambdaBody::Expr(x) => self.expr(x),
            },
            _ => {}
        }
        (self.on_expr)(e);
    }
}
