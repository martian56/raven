//! Stable text dump of a `HirProgram` for snapshot testing.
//!
//! The format mirrors the AST pretty printer: an S-expression style
//! tree where each node prints its kind and fields, one nesting level
//! per indentation. Spans are not printed (they would cause churn
//! whenever a corpus file is touched). Types ARE printed because they
//! are part of the HIR's contract with later passes.

use std::fmt::Write;

use super::expr::{HirArm, HirBinaryOp, HirBlock, HirExpr, HirExprKind, HirUnaryOp, InterpolPart};
use super::pattern::{HirFieldPat, HirLiteralPat, HirPattern, HirPatternKind};
use super::stmt::{HirAssignTarget, HirStmt, HirStmtKind};
use super::HirProgram;
use super::{HirItem, HirItemKind};
use crate::hir::decl::{HirEnum, HirFn, HirImpl, HirStruct, HirTrait, HirVariant};

/// Render an entire HIR program as a multi line S expression string.
pub fn pretty_program(program: &HirProgram) -> String {
    let mut out = String::new();
    writeln!(&mut out, "(hir").unwrap();
    for item in &program.items {
        pretty_item(&mut out, item, 1);
    }
    writeln!(&mut out, ")").unwrap();
    out
}

fn indent(buf: &mut String, depth: usize) {
    for _ in 0..depth {
        buf.push_str("  ");
    }
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn pretty_item(buf: &mut String, item: &HirItem, depth: usize) {
    indent(buf, depth);
    match &item.kind {
        HirItemKind::Function(f) => pretty_fn(buf, f, depth, "fn"),
        HirItemKind::Struct(s) => pretty_struct(buf, s, depth),
        HirItemKind::Trait(t) => pretty_trait(buf, t, depth),
        HirItemKind::Impl(i) => pretty_impl(buf, i, depth),
        HirItemKind::Enum(e) => pretty_enum(buf, e, depth),
        HirItemKind::Const { name, ty, value } => {
            writeln!(buf, "(const {} ty={}", quote(name), ty).unwrap();
            pretty_expr(buf, value, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirItemKind::Let { name, ty, init } => {
            writeln!(buf, "(let-item {} ty={}", quote(name), ty).unwrap();
            if let Some(e) = init {
                pretty_expr(buf, e, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirItemKind::Extern(e) => {
            writeln!(buf, "(extern {}", quote(&e.abi)).unwrap();
            for item in &e.items {
                indent(buf, depth + 1);
                write!(buf, "(extern-fn {} params=(", quote(&item.name)).unwrap();
                for (i, p) in item.params.iter().enumerate() {
                    if i > 0 {
                        buf.push(' ');
                    }
                    write!(buf, "{}", p).unwrap();
                }
                writeln!(buf, ") ret={})", item.ret).unwrap();
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirItemKind::Opaque(s) => {
            writeln!(buf, "(opaque {})", quote(s)).unwrap();
        }
    }
}

fn pretty_fn(buf: &mut String, f: &HirFn, depth: usize, kw: &str) {
    write!(buf, "({} {}", kw, quote(&f.name)).unwrap();
    buf.push_str(" params=(");
    for (i, (name, ty, _)) in f.params.iter().enumerate() {
        if i > 0 {
            buf.push(' ');
        }
        write!(buf, "{}:{}", quote(name), ty).unwrap();
    }
    buf.push(')');
    write!(buf, " ret={}", f.ret).unwrap();
    buf.push('\n');
    if let Some(body) = &f.body {
        pretty_block(buf, body, depth + 1, "body");
    } else {
        indent(buf, depth + 1);
        buf.push_str("(body-none)\n");
    }
    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_struct(buf: &mut String, s: &HirStruct, depth: usize) {
    writeln!(buf, "(struct {}", quote(&s.name)).unwrap();
    for (name, ty, _) in &s.fields {
        indent(buf, depth + 1);
        writeln!(buf, "(field {} {})", quote(name), ty).unwrap();
    }
    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_trait(buf: &mut String, t: &HirTrait, depth: usize) {
    writeln!(buf, "(trait {}", quote(&t.name)).unwrap();
    for m in &t.methods {
        indent(buf, depth + 1);
        pretty_fn(buf, m, depth + 1, "method");
    }
    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_impl(buf: &mut String, i: &HirImpl, depth: usize) {
    write!(buf, "(impl self={}", quote(&i.self_name)).unwrap();
    if let Some(t) = &i.trait_name {
        write!(buf, " trait={}", quote(t)).unwrap();
    }
    buf.push('\n');
    for m in &i.methods {
        indent(buf, depth + 1);
        pretty_fn(buf, m, depth + 1, "method");
    }
    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_enum(buf: &mut String, e: &HirEnum, depth: usize) {
    writeln!(buf, "(enum {}", quote(&e.name)).unwrap();
    for v in &e.variants {
        indent(buf, depth + 1);
        pretty_variant(buf, v, depth + 1);
    }
    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_variant(buf: &mut String, v: &HirVariant, depth: usize) {
    write!(buf, "(variant {}", quote(&v.name)).unwrap();
    if v.fields.is_empty() {
        buf.push_str(")\n");
        return;
    }
    buf.push('\n');
    for (name, ty, _) in &v.fields {
        indent(buf, depth + 1);
        writeln!(buf, "(field {} {})", quote(name), ty).unwrap();
    }
    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_block(buf: &mut String, block: &HirBlock, depth: usize, label: &str) {
    indent(buf, depth);
    writeln!(buf, "({} ty={}", label, block.ty).unwrap();
    for s in &block.stmts {
        pretty_stmt(buf, s, depth + 1);
    }
    if let Some(tail) = &block.tail {
        indent(buf, depth + 1);
        buf.push_str("(tail\n");
        pretty_expr(buf, tail, depth + 2);
        indent(buf, depth + 1);
        buf.push_str(")\n");
    }
    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_stmt(buf: &mut String, stmt: &HirStmt, depth: usize) {
    indent(buf, depth);
    match &stmt.kind {
        HirStmtKind::Let { name, ty, init } => {
            writeln!(buf, "(let {} ty={}", quote(name), ty).unwrap();
            pretty_expr(buf, init, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirStmtKind::Expr(e) => {
            buf.push_str("(stmt-expr\n");
            pretty_expr(buf, e, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirStmtKind::Assign { target, value } => {
            buf.push_str("(assign\n");
            indent(buf, depth + 1);
            pretty_assign_target(buf, target, depth + 1);
            buf.push('\n');
            pretty_expr(buf, value, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirStmtKind::Defer(e) => {
            buf.push_str("(defer\n");
            pretty_expr(buf, e, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
    }
}

fn pretty_assign_target(buf: &mut String, target: &HirAssignTarget, depth: usize) {
    match target {
        HirAssignTarget::Ident { name, .. } => {
            write!(buf, "(target-ident {})", quote(name)).unwrap();
        }
        HirAssignTarget::Field { recv, name } => {
            buf.push_str("(target-field name=");
            buf.push_str(&quote(name));
            buf.push('\n');
            pretty_expr(buf, recv, depth + 1);
            indent(buf, depth);
            buf.push(')');
        }
        HirAssignTarget::Index { recv, index } => {
            buf.push_str("(target-index\n");
            pretty_expr(buf, recv, depth + 1);
            pretty_expr(buf, index, depth + 1);
            indent(buf, depth);
            buf.push(')');
        }
    }
}

fn pretty_expr(buf: &mut String, expr: &HirExpr, depth: usize) {
    indent(buf, depth);
    match &expr.kind {
        HirExprKind::Int(i) => writeln!(buf, "(int {} ty={})", i, expr.ty).unwrap(),
        HirExprKind::Float(v) => writeln!(buf, "(float {} ty={})", v, expr.ty).unwrap(),
        HirExprKind::Bool(b) => writeln!(buf, "(bool {} ty={})", b, expr.ty).unwrap(),
        HirExprKind::Str(s) => writeln!(buf, "(str {} ty={})", quote(s), expr.ty).unwrap(),
        HirExprKind::Char(c) => {
            writeln!(buf, "(char {} ty={})", quote(&c.to_string()), expr.ty).unwrap()
        }
        HirExprKind::CStr(s) => writeln!(buf, "(cstr {} ty={})", quote(s), expr.ty).unwrap(),
        HirExprKind::Unit => writeln!(buf, "(unit ty={})", expr.ty).unwrap(),
        HirExprKind::Ident(n) => writeln!(buf, "(ident {} ty={})", quote(n), expr.ty).unwrap(),
        HirExprKind::SelfValue => writeln!(buf, "(self ty={})", expr.ty).unwrap(),
        HirExprKind::Array(items) => {
            writeln!(buf, "(array ty={}", expr.ty).unwrap();
            for it in items {
                pretty_expr(buf, it, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::StructLit { name, fields } => {
            writeln!(buf, "(struct-lit {} ty={}", quote(name), expr.ty).unwrap();
            for (fname, fval) in fields {
                indent(buf, depth + 1);
                writeln!(buf, "(field {}", quote(fname)).unwrap();
                pretty_expr(buf, fval, depth + 2);
                indent(buf, depth + 1);
                buf.push_str(")\n");
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Paren(inner) => {
            writeln!(buf, "(paren ty={}", expr.ty).unwrap();
            pretty_expr(buf, inner, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Block(b) => pretty_block(buf, b, depth, "block"),
        HirExprKind::Unary { op, operand } => {
            writeln!(buf, "(unary {} ty={}", unop(*op), expr.ty).unwrap();
            pretty_expr(buf, operand, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Binary { op, lhs, rhs } => {
            writeln!(buf, "(binary {} ty={}", binop(*op), expr.ty).unwrap();
            pretty_expr(buf, lhs, depth + 1);
            pretty_expr(buf, rhs, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Call { callee, args } => {
            writeln!(buf, "(call ty={}", expr.ty).unwrap();
            pretty_expr(buf, callee, depth + 1);
            for a in args {
                pretty_expr(buf, a, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::MethodCall {
            receiver,
            name,
            args,
        } => {
            writeln!(buf, "(method-call {} ty={}", quote(name), expr.ty).unwrap();
            pretty_expr(buf, receiver, depth + 1);
            for a in args {
                pretty_expr(buf, a, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::AssocCall {
            self_ty,
            name,
            args,
        } => {
            writeln!(
                buf,
                "(assoc-call {} on={} ty={}",
                quote(name),
                self_ty,
                expr.ty
            )
            .unwrap();
            for a in args {
                pretty_expr(buf, a, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Field { receiver, name } => {
            writeln!(buf, "(field-access {} ty={}", quote(name), expr.ty).unwrap();
            pretty_expr(buf, receiver, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Index { receiver, index } => {
            writeln!(buf, "(index ty={}", expr.ty).unwrap();
            pretty_expr(buf, receiver, depth + 1);
            pretty_expr(buf, index, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::If {
            cond,
            then_block,
            else_block,
        } => {
            writeln!(buf, "(if ty={}", expr.ty).unwrap();
            pretty_expr(buf, cond, depth + 1);
            pretty_block(buf, then_block, depth + 1, "then");
            if let Some(e) = else_block {
                pretty_block(buf, e, depth + 1, "else");
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Match { scrutinee, arms } => {
            writeln!(buf, "(match ty={}", expr.ty).unwrap();
            pretty_expr(buf, scrutinee, depth + 1);
            for a in arms {
                pretty_arm(buf, a, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Loop(b) => {
            writeln!(buf, "(loop ty={}", expr.ty).unwrap();
            pretty_block(buf, b, depth + 1, "body");
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::While { cond, body } => {
            writeln!(buf, "(while ty={}", expr.ty).unwrap();
            pretty_expr(buf, cond, depth + 1);
            pretty_block(buf, body, depth + 1, "body");
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Return(value) => {
            writeln!(buf, "(return ty={}", expr.ty).unwrap();
            if let Some(v) = value {
                pretty_expr(buf, v, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Break(value) => {
            writeln!(buf, "(break ty={}", expr.ty).unwrap();
            if let Some(v) = value {
                pretty_expr(buf, v, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Continue => writeln!(buf, "(continue ty={})", expr.ty).unwrap(),
        HirExprKind::Interpolate(parts) => {
            writeln!(buf, "(interpolate ty={}", expr.ty).unwrap();
            for p in parts {
                match p {
                    InterpolPart::Text(t) => {
                        indent(buf, depth + 1);
                        writeln!(buf, "(text {})", quote(t)).unwrap();
                    }
                    InterpolPart::Expr(e) => {
                        indent(buf, depth + 1);
                        buf.push_str("(part\n");
                        pretty_expr(buf, e, depth + 2);
                        indent(buf, depth + 1);
                        buf.push_str(")\n");
                    }
                }
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::RangeNew {
            start,
            end,
            inclusive,
        } => {
            writeln!(buf, "(range-new inclusive={} ty={}", inclusive, expr.ty).unwrap();
            pretty_expr(buf, start, depth + 1);
            pretty_expr(buf, end, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::IterNew(inner) => {
            writeln!(buf, "(iter-new ty={}", expr.ty).unwrap();
            pretty_expr(buf, inner, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::IterNext(inner) => {
            writeln!(buf, "(iter-next ty={}", expr.ty).unwrap();
            pretty_expr(buf, inner, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::OkCtor(inner) => {
            writeln!(buf, "(ok ty={}", expr.ty).unwrap();
            pretty_expr(buf, inner, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::ErrCtor(inner) => {
            writeln!(buf, "(err ty={}", expr.ty).unwrap();
            pretty_expr(buf, inner, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::SomeCtor(inner) => {
            writeln!(buf, "(some ty={}", expr.ty).unwrap();
            pretty_expr(buf, inner, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::NoneCtor => writeln!(buf, "(none ty={})", expr.ty).unwrap(),
        HirExprKind::EnumCreate { variant, args } => {
            writeln!(buf, "(enum-create variant={} ty={}", variant, expr.ty).unwrap();
            for a in args {
                pretty_expr(buf, a, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::Lambda { params, ret, body } => {
            write!(buf, "(lambda ret={} params=(", ret).unwrap();
            for (i, (name, ty, _)) in params.iter().enumerate() {
                if i > 0 {
                    buf.push(' ');
                }
                write!(buf, "{}:{}", quote(name), ty).unwrap();
            }
            buf.push(')');
            writeln!(buf, " ty={}", expr.ty).unwrap();
            pretty_block(buf, body, depth + 1, "body");
            indent(buf, depth);
            buf.push_str(")\n");
        }
        HirExprKind::DynCoerce {
            trait_name,
            value,
            concrete_ty,
            ..
        } => {
            writeln!(
                buf,
                "(dyn-coerce trait={} from={} ty={}",
                trait_name, concrete_ty, expr.ty
            )
            .unwrap();
            pretty_expr(buf, value, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
    }
}

fn pretty_arm(buf: &mut String, arm: &HirArm, depth: usize) {
    indent(buf, depth);
    buf.push_str("(arm\n");
    indent(buf, depth + 1);
    buf.push_str("(pat ");
    pretty_pattern(buf, &arm.pattern, depth + 1);
    buf.push_str(")\n");
    if let Some(g) = &arm.guard {
        indent(buf, depth + 1);
        buf.push_str("(guard\n");
        pretty_expr(buf, g, depth + 2);
        indent(buf, depth + 1);
        buf.push_str(")\n");
    }
    pretty_expr(buf, &arm.body, depth + 1);
    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_pattern(buf: &mut String, pat: &HirPattern, _depth: usize) {
    match &pat.kind {
        HirPatternKind::Wildcard => buf.push('_'),
        HirPatternKind::Literal(lit) => match lit {
            HirLiteralPat::Int(i) => write!(buf, "{}", i).unwrap(),
            HirLiteralPat::Float(v) => write!(buf, "{}", v).unwrap(),
            HirLiteralPat::Bool(b) => write!(buf, "{}", b).unwrap(),
            HirLiteralPat::Str(s) => buf.push_str(&quote(s)),
            HirLiteralPat::Char(c) => buf.push_str(&quote(&c.to_string())),
        },
        HirPatternKind::Binding(name) => buf.push_str(name),
        HirPatternKind::Constructor { name, elements } => {
            if let Some(n) = name {
                buf.push_str(n);
            }
            buf.push('(');
            for (i, e) in elements.iter().enumerate() {
                if i > 0 {
                    buf.push_str(", ");
                }
                pretty_pattern(buf, e, _depth);
            }
            buf.push(')');
        }
        HirPatternKind::Struct { name, fields } => {
            buf.push_str(name);
            buf.push_str(" {");
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    buf.push_str(", ");
                }
                pretty_field_pat(buf, f);
            }
            buf.push('}');
        }
        HirPatternKind::Range { lo, hi, inclusive } => {
            let dots = if *inclusive { "..=" } else { ".." };
            write!(buf, "{}{}{}", lo, dots, hi).unwrap();
        }
    }
}

fn pretty_field_pat(buf: &mut String, fp: &HirFieldPat) {
    buf.push_str(&fp.name);
    if let Some(p) = &fp.pattern {
        buf.push_str(": ");
        pretty_pattern(buf, p, 0);
    }
}

fn unop(op: HirUnaryOp) -> &'static str {
    match op {
        HirUnaryOp::Neg => "neg",
        HirUnaryOp::Not => "not",
        HirUnaryOp::Ref => "ref",
    }
}

fn binop(op: HirBinaryOp) -> &'static str {
    match op {
        HirBinaryOp::Add => "+",
        HirBinaryOp::Sub => "-",
        HirBinaryOp::Mul => "*",
        HirBinaryOp::Div => "/",
        HirBinaryOp::Mod => "%",
        HirBinaryOp::Eq => "==",
        HirBinaryOp::Ne => "!=",
        HirBinaryOp::Lt => "<",
        HirBinaryOp::Le => "<=",
        HirBinaryOp::Gt => ">",
        HirBinaryOp::Ge => ">=",
        HirBinaryOp::And => "&&",
        HirBinaryOp::Or => "||",
        HirBinaryOp::BitAnd => "&",
        HirBinaryOp::BitOr => "|",
        HirBinaryOp::BitXor => "^",
        HirBinaryOp::Shl => "<<",
        HirBinaryOp::Shr => ">>",
    }
}
