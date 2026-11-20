//! Copy a function with shifted span offsets (passes key on offsets) and
//! substituted type names.

use std::collections::HashMap;

use crate::span::Span;

use super::decl::{Function, FunctionBody, GenericParam, Param};
use super::expr::{Block, ElseBranch, Expr, ExprKind, LambdaBody, MatchArm, StrFragment};
use super::pattern::{Pattern, PatternKind};
use super::stmt::{Stmt, StmtKind};
use super::ty::{Type, TypeKind, TypePath};

pub struct Respan<'a> {
    copy: usize,
    subst: &'a HashMap<String, Type>,
}

impl<'a> Respan<'a> {
    pub fn new(copy: usize, subst: &'a HashMap<String, Type>) -> Self {
        Respan { copy, subst }
    }

    fn span(&self, s: &mut Span) {
        *s = s.in_copy(self.copy);
    }

    pub fn function(&self, f: &mut Function) {
        self.span(&mut f.span);
        for g in &mut f.generics {
            self.generic(g);
        }
        for p in &mut f.params {
            self.param(p);
        }
        if let Some(t) = &mut f.ret {
            self.ty(t);
        }
        match &mut f.body {
            FunctionBody::Block(b) => self.block(b),
            FunctionBody::Expr(e) => self.expr(e),
            FunctionBody::None => {}
        }
    }

    fn generic(&self, g: &mut GenericParam) {
        self.span(&mut g.span);
        for b in &mut g.bounds {
            self.path(b);
        }
    }

    fn param(&self, p: &mut Param) {
        self.span(&mut p.span);
        self.ty(&mut p.ty);
    }

    fn ty(&self, t: &mut Type) {
        if let TypeKind::Path(p) = &t.kind {
            if p.segments.len() == 1 && p.segments[0].generics.is_empty() {
                if let Some(rep) = self.subst.get(&p.segments[0].name) {
                    let span = t.span.clone();
                    *t = rep.clone();
                    t.span = span;
                    self.span(&mut t.span);
                    return;
                }
            }
        }
        self.span(&mut t.span);
        match &mut t.kind {
            TypeKind::Path(p) | TypeKind::Dyn(p) => self.path(p),
            TypeKind::Optional(inner) => self.ty(inner),
            TypeKind::Unit => {}
            TypeKind::Function { params, ret } => {
                for p in params {
                    self.ty(p);
                }
                self.ty(ret);
            }
        }
    }

    fn path(&self, p: &mut TypePath) {
        self.span(&mut p.span);
        for seg in &mut p.segments {
            self.span(&mut seg.span);
            for g in &mut seg.generics {
                self.ty(g);
            }
        }
    }

    fn block(&self, b: &mut Block) {
        self.span(&mut b.span);
        for s in &mut b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &mut b.trailing {
            self.expr(t);
        }
    }

    fn stmt(&self, s: &mut Stmt) {
        self.span(&mut s.span);
        match &mut s.kind {
            StmtKind::Let { ty, init, .. } => {
                if let Some(t) = ty {
                    self.ty(t);
                }
                if let Some(e) = init {
                    self.expr(e);
                }
            }
            StmtKind::Return(e) | StmtKind::Break(e) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            StmtKind::Continue => {}
            StmtKind::LetElse {
                pattern,
                init,
                else_block,
                ..
            } => {
                self.pattern(pattern);
                self.expr(init);
                self.block(else_block);
            }
            StmtKind::LetTuple { pattern, init, .. } => {
                self.pattern(pattern);
                self.expr(init);
            }
            StmtKind::Defer(e) | StmtKind::Spawn(e) | StmtKind::Expr(e) => self.expr(e),
            StmtKind::Assign { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
        }
    }

    fn pattern(&self, p: &mut Pattern) {
        self.span(&mut p.span);
        match &mut p.kind {
            PatternKind::Tuple { elements, .. } => {
                for e in elements {
                    self.pattern(e);
                }
            }
            PatternKind::Struct { fields, .. } => {
                for f in fields {
                    self.span(&mut f.span);
                    if let Some(p) = &mut f.pattern {
                        self.pattern(p);
                    }
                }
            }
            PatternKind::Wildcard
            | PatternKind::Literal(_)
            | PatternKind::Ident(_)
            | PatternKind::Range { .. } => {}
        }
    }

    fn arm(&self, a: &mut MatchArm) {
        self.span(&mut a.span);
        self.pattern(&mut a.pattern);
        if let Some(g) = &mut a.guard {
            self.expr(g);
        }
        self.expr(&mut a.body);
    }

    fn exprs(&self, es: &mut [Expr]) {
        for e in es {
            self.expr(e);
        }
    }

    fn tys(&self, ts: &mut [Type]) {
        for t in ts {
            self.ty(t);
        }
    }

    pub fn expr(&self, e: &mut Expr) {
        self.span(&mut e.span);
        match &mut e.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::BlockStr(_)
            | ExprKind::Char(_)
            | ExprKind::CStr(_)
            | ExprKind::SelfLower
            | ExprKind::SelfUpper
            | ExprKind::MacroCall(_) => {}
            ExprKind::InterpolatedString(frags) => {
                for f in frags {
                    if let StrFragment::Expr(e) = f {
                        self.expr(e);
                    }
                }
            }
            ExprKind::Ident { generics, .. } => self.tys(generics),
            ExprKind::StructLit {
                generics, fields, ..
            } => {
                self.tys(generics);
                for f in fields {
                    self.span(&mut f.span);
                    self.expr(&mut f.value);
                }
            }
            ExprKind::Array(xs) | ExprKind::SetLit(xs) | ExprKind::Tuple(xs) => self.exprs(xs),
            ExprKind::MapLit(pairs) => {
                for (k, v) in pairs {
                    self.expr(k);
                    self.expr(v);
                }
            }
            ExprKind::Paren(x) | ExprKind::Try(x) | ExprKind::NamedArg { value: x, .. } => {
                self.expr(x)
            }
            ExprKind::Cast { value, ty } => {
                self.expr(value);
                self.ty(ty);
            }
            ExprKind::Comprehension {
                key,
                value,
                pattern,
                iter,
                cond,
                ..
            } => {
                if let Some(k) = key {
                    self.expr(k);
                }
                self.expr(value);
                self.pattern(pattern);
                self.expr(iter);
                if let Some(c) = cond {
                    self.expr(c);
                }
            }
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
                self.exprs(args);
            }
            ExprKind::MethodCall {
                receiver,
                generics,
                args,
                ..
            } => {
                self.expr(receiver);
                self.tys(generics);
                self.exprs(args);
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
                    match eb.as_mut() {
                        ElseBranch::If(e) => self.expr(e),
                        ElseBranch::Block(b) => self.block(b),
                    }
                }
            }
            ExprKind::IfLet {
                pattern,
                scrutinee,
                then_branch,
                else_branch,
            } => {
                self.pattern(pattern);
                self.expr(scrutinee);
                self.block(then_branch);
                if let Some(eb) = else_branch {
                    match eb.as_mut() {
                        ElseBranch::If(e) => self.expr(e),
                        ElseBranch::Block(b) => self.block(b),
                    }
                }
            }
            ExprKind::WhileLet {
                pattern,
                scrutinee,
                body,
            } => {
                self.pattern(pattern);
                self.expr(scrutinee);
                self.block(body);
            }
            ExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for a in arms {
                    self.arm(a);
                }
            }
            ExprKind::While { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            ExprKind::For {
                pattern,
                iter,
                body,
            } => {
                self.pattern(pattern);
                self.expr(iter);
                self.block(body);
            }
            ExprKind::Lambda {
                params, ret, body, ..
            } => {
                for p in params {
                    self.span(&mut p.span);
                    if let Some(t) = &mut p.ty {
                        self.ty(t);
                    }
                }
                if let Some(t) = ret {
                    self.ty(t);
                }
                match body {
                    LambdaBody::Block(b) => self.block(b),
                    LambdaBody::Expr(x) => self.expr(x),
                }
            }
        }
    }
}
