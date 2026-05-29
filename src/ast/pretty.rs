//! S expression style pretty printer for AST nodes.
//!
//! Used by the golden snapshot tests in `tests/parser_golden.rs`. The
//! format is deliberately verbose and stable: every node prints its
//! kind followed by its fields, one level of indentation per nesting.
//! Spans are NOT printed because they would create churn whenever a
//! corpus file is touched; the lexer's own tests cover span
//! correctness.

use std::fmt::Write;

use super::{
    AssignOp, BinaryOp, Block, Decl, DeclKind, ElseBranch, Expr, ExprKind, File, FunctionBody,
    ImportSource, LambdaBody, LiteralPattern, Pattern, PatternKind, Stmt, StmtKind, StrFragment,
    Type, TypeKind, TypePath, UnaryOp, VariantPayload,
};

/// Render a [`File`] as a multi line S expression string.
pub fn pretty_file(file: &File) -> String {
    let mut out = String::new();
    writeln!(&mut out, "(file").unwrap();
    for item in &file.items {
        pretty_decl(&mut out, item, 1);
    }
    writeln!(&mut out, ")").unwrap();
    out
}

fn indent(buf: &mut String, depth: usize) {
    for _ in 0..depth {
        buf.push_str("  ");
    }
}

fn pretty_decl(buf: &mut String, decl: &Decl, depth: usize) {
    indent(buf, depth);
    match &decl.kind {
        DeclKind::Function(f) => {
            write!(buf, "(fn {}", quote(&f.name)).unwrap();
            if !f.generics.is_empty() {
                buf.push_str(" generics=(");
                for (i, g) in f.generics.iter().enumerate() {
                    if i > 0 {
                        buf.push(' ');
                    }
                    write!(buf, "{}", quote(&g.name)).unwrap();
                    if !g.bounds.is_empty() {
                        buf.push(':');
                        for (j, b) in g.bounds.iter().enumerate() {
                            if j > 0 {
                                buf.push('+');
                            }
                            buf.push_str(&pretty_type_path(b));
                        }
                    }
                }
                buf.push(')');
            }
            buf.push_str(" params=(");
            for (i, p) in f.params.iter().enumerate() {
                if i > 0 {
                    buf.push(' ');
                }
                write!(buf, "{}:{}", quote(&p.name), pretty_type(&p.ty)).unwrap();
            }
            buf.push(')');
            if let Some(ret) = &f.ret {
                write!(buf, " ret={}", pretty_type(ret)).unwrap();
            }
            buf.push('\n');
            match &f.body {
                FunctionBody::Block(b) => pretty_block(buf, b, depth + 1, "body"),
                FunctionBody::Expr(e) => {
                    indent(buf, depth + 1);
                    buf.push_str("(body-expr\n");
                    pretty_expr(buf, e, depth + 2);
                    indent(buf, depth + 1);
                    buf.push_str(")\n");
                }
                FunctionBody::None => {
                    indent(buf, depth + 1);
                    buf.push_str("(body-none)\n");
                }
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        DeclKind::Struct(s) => {
            write!(buf, "(struct {}", quote(&s.name)).unwrap();
            if !s.generics.is_empty() {
                buf.push_str(" generics=(");
                for (i, g) in s.generics.iter().enumerate() {
                    if i > 0 {
                        buf.push(' ');
                    }
                    buf.push_str(&quote(&g.name));
                }
                buf.push(')');
            }
            buf.push('\n');
            for fld in &s.fields {
                indent(buf, depth + 1);
                writeln!(
                    buf,
                    "(field {}: {})",
                    quote(&fld.name),
                    pretty_type(&fld.ty)
                )
                .unwrap();
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        DeclKind::Trait(t) => {
            writeln!(buf, "(trait {}", quote(&t.name)).unwrap();
            for m in &t.members {
                let sub = Decl {
                    kind: DeclKind::Function(m.clone()),
                    span: m.span.clone(),
                };
                pretty_decl(buf, &sub, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        DeclKind::Impl(i) => {
            buf.push_str("(impl");
            if let Some(t) = &i.for_type {
                write!(
                    buf,
                    " trait={} for={}",
                    pretty_type_path(&i.trait_or_type),
                    pretty_type_path(t)
                )
                .unwrap();
            } else {
                write!(buf, " type={}", pretty_type_path(&i.trait_or_type)).unwrap();
            }
            buf.push('\n');
            for it in &i.items {
                let sub = Decl {
                    kind: DeclKind::Function(it.clone()),
                    span: it.span.clone(),
                };
                pretty_decl(buf, &sub, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        DeclKind::Enum(e) => {
            writeln!(buf, "(enum {}", quote(&e.name)).unwrap();
            for v in &e.variants {
                indent(buf, depth + 1);
                match &v.payload {
                    VariantPayload::Unit => writeln!(buf, "(variant {})", quote(&v.name)).unwrap(),
                    VariantPayload::Tuple(tys) => {
                        write!(buf, "(variant {} tuple(", quote(&v.name)).unwrap();
                        for (j, t) in tys.iter().enumerate() {
                            if j > 0 {
                                buf.push(' ');
                            }
                            buf.push_str(&pretty_type(t));
                        }
                        buf.push_str("))\n");
                    }
                    VariantPayload::Struct(flds) => {
                        write!(buf, "(variant {} struct(", quote(&v.name)).unwrap();
                        for (j, f) in flds.iter().enumerate() {
                            if j > 0 {
                                buf.push(' ');
                            }
                            write!(buf, "{}:{}", quote(&f.name), pretty_type(&f.ty)).unwrap();
                        }
                        buf.push_str("))\n");
                    }
                }
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        DeclKind::Extern(e) => {
            writeln!(buf, "(extern {}", quote(&e.abi)).unwrap();
            for it in &e.items {
                indent(buf, depth + 1);
                write!(buf, "(extern-fn {}", quote(&it.name)).unwrap();
                buf.push_str(" params=(");
                for (i, p) in it.params.iter().enumerate() {
                    if i > 0 {
                        buf.push(' ');
                    }
                    write!(buf, "{}:{}", quote(&p.name), pretty_type(&p.ty)).unwrap();
                }
                buf.push(')');
                if let Some(r) = &it.ret {
                    write!(buf, " ret={}", pretty_type(r)).unwrap();
                }
                buf.push_str(")\n");
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        DeclKind::Import(im) => {
            write!(buf, "(import").unwrap();
            match &im.source {
                ImportSource::Std(parts) => {
                    write!(buf, " std=({})", parts.join(" ")).unwrap();
                }
                ImportSource::Quoted(s) => {
                    write!(buf, " quoted={}", quote(s)).unwrap();
                }
            }
            if let Some(a) = &im.alias {
                write!(buf, " alias={}", quote(a)).unwrap();
            }
            if !im.selectors.is_empty() {
                write!(buf, " selectors=({})", im.selectors.join(" ")).unwrap();
            }
            buf.push_str(")\n");
        }
        DeclKind::Const(c) => {
            writeln!(buf, "(const {}: {} =", quote(&c.name), pretty_type(&c.ty)).unwrap();
            pretty_expr(buf, &c.value, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        DeclKind::Let(l) => {
            write!(buf, "(let {}", quote(&l.name)).unwrap();
            if let Some(t) = &l.ty {
                write!(buf, ": {}", pretty_type(t)).unwrap();
            }
            buf.push('\n');
            if let Some(e) = &l.init {
                pretty_expr(buf, e, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
    }
}

fn pretty_block(buf: &mut String, block: &Block, depth: usize, label: &str) {
    indent(buf, depth);
    writeln!(buf, "({}", label).unwrap();
    for s in &block.stmts {
        pretty_stmt(buf, s, depth + 1);
    }
    if let Some(t) = &block.trailing {
        indent(buf, depth + 1);
        buf.push_str("(trailing\n");
        pretty_expr(buf, t, depth + 2);
        indent(buf, depth + 1);
        buf.push_str(")\n");
    }
    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_stmt(buf: &mut String, stmt: &Stmt, depth: usize) {
    indent(buf, depth);
    match &stmt.kind {
        StmtKind::Let { name, ty, init } => {
            write!(buf, "(let-stmt {}", quote(name)).unwrap();
            if let Some(t) = ty {
                write!(buf, ": {}", pretty_type(t)).unwrap();
            }
            buf.push('\n');
            if let Some(e) = init {
                pretty_expr(buf, e, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        StmtKind::Return(e) => {
            buf.push_str("(return\n");
            if let Some(e) = e {
                pretty_expr(buf, e, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        StmtKind::Break(e) => {
            buf.push_str("(break\n");
            if let Some(e) = e {
                pretty_expr(buf, e, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        StmtKind::Continue => {
            buf.push_str("(continue)\n");
        }
        StmtKind::Defer(e) => {
            buf.push_str("(defer\n");
            pretty_expr(buf, e, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        StmtKind::Assign { target, op, value } => {
            writeln!(buf, "(assign op={}", assign_op(*op)).unwrap();
            pretty_expr(buf, target, depth + 1);
            pretty_expr(buf, value, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        StmtKind::Expr(e) => {
            buf.push_str("(expr-stmt\n");
            pretty_expr(buf, e, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
    }
}

fn pretty_expr(buf: &mut String, expr: &Expr, depth: usize) {
    indent(buf, depth);
    match &expr.kind {
        ExprKind::Int(n) => writeln!(buf, "(int {})", n).unwrap(),
        ExprKind::Float(v) => writeln!(buf, "(float {})", v).unwrap(),
        ExprKind::Bool(b) => writeln!(buf, "(bool {})", b).unwrap(),
        ExprKind::Str(s) => writeln!(buf, "(str {})", quote(s)).unwrap(),
        ExprKind::InterpolatedString(fragments) => {
            buf.push_str("(interpolated\n");
            for frag in fragments {
                indent(buf, depth + 1);
                match frag {
                    StrFragment::Literal(s) => writeln!(buf, "(lit {})", quote(s)).unwrap(),
                    StrFragment::Expr(e) => {
                        buf.push_str("(frag\n");
                        pretty_expr(buf, e, depth + 2);
                        indent(buf, depth + 1);
                        buf.push_str(")\n");
                    }
                }
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::BlockStr(s) => writeln!(buf, "(blockstr {})", quote(s)).unwrap(),
        ExprKind::Char(c) => writeln!(buf, "(char {:?})", c).unwrap(),
        ExprKind::CStr(s) => writeln!(buf, "(cstr {})", quote(s)).unwrap(),
        ExprKind::SelfLower => writeln!(buf, "(self)").unwrap(),
        ExprKind::SelfUpper => writeln!(buf, "(Self)").unwrap(),
        ExprKind::Ident { name, generics } => {
            write!(buf, "(ident {}", quote(name)).unwrap();
            if !generics.is_empty() {
                buf.push_str(" generics=(");
                for (i, g) in generics.iter().enumerate() {
                    if i > 0 {
                        buf.push(' ');
                    }
                    buf.push_str(&pretty_type(g));
                }
                buf.push(')');
            }
            buf.push_str(")\n");
        }
        ExprKind::StructLit {
            name,
            generics,
            fields,
        } => {
            write!(buf, "(struct-lit {}", quote(name)).unwrap();
            if !generics.is_empty() {
                buf.push_str(" generics=(");
                for (i, g) in generics.iter().enumerate() {
                    if i > 0 {
                        buf.push(' ');
                    }
                    buf.push_str(&pretty_type(g));
                }
                buf.push(')');
            }
            buf.push('\n');
            for f in fields {
                indent(buf, depth + 1);
                writeln!(buf, "(field-init {}", quote(&f.name)).unwrap();
                pretty_expr(buf, &f.value, depth + 2);
                indent(buf, depth + 1);
                buf.push_str(")\n");
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Array(items) => {
            buf.push_str("(array\n");
            for it in items {
                pretty_expr(buf, it, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Tuple(items) => {
            buf.push_str("(tuple\n");
            for it in items {
                pretty_expr(buf, it, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::SetLit(items) => {
            buf.push_str("(set\n");
            for it in items {
                pretty_expr(buf, it, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::MapLit(pairs) => {
            buf.push_str("(map\n");
            for (k, v) in pairs {
                indent(buf, depth + 1);
                buf.push_str("(pair\n");
                pretty_expr(buf, k, depth + 2);
                pretty_expr(buf, v, depth + 2);
                indent(buf, depth + 1);
                buf.push_str(")\n");
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Paren(inner) => {
            buf.push_str("(paren\n");
            pretty_expr(buf, inner, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Block(b) => {
            pretty_block(buf, b, depth, "block");
            // pretty_block adds its own indent; remove the one we already
            // wrote.
            // (Pretty_block was called with its own indent which doubles
            // up; we strip back by using "block" as the outer label.)
        }
        ExprKind::Unary { op, operand } => {
            writeln!(buf, "(unary {}", unary_op(*op)).unwrap();
            pretty_expr(buf, operand, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Binary { op, lhs, rhs } => {
            writeln!(buf, "(binary {}", binary_op(*op)).unwrap();
            pretty_expr(buf, lhs, depth + 1);
            pretty_expr(buf, rhs, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Range {
            start,
            end,
            inclusive,
        } => {
            writeln!(buf, "(range inclusive={}", inclusive).unwrap();
            pretty_expr(buf, start, depth + 1);
            pretty_expr(buf, end, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Call { callee, args } => {
            buf.push_str("(call\n");
            pretty_expr(buf, callee, depth + 1);
            for a in args {
                pretty_expr(buf, a, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::MethodCall {
            receiver,
            name,
            generics,
            args,
        } => {
            write!(buf, "(method-call {}", quote(name)).unwrap();
            if !generics.is_empty() {
                buf.push_str(" generics=(");
                for (i, g) in generics.iter().enumerate() {
                    if i > 0 {
                        buf.push(' ');
                    }
                    buf.push_str(&pretty_type(g));
                }
                buf.push(')');
            }
            buf.push('\n');
            pretty_expr(buf, receiver, depth + 1);
            for a in args {
                pretty_expr(buf, a, depth + 1);
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Field { receiver, name } => {
            writeln!(buf, "(field {}", quote(name)).unwrap();
            pretty_expr(buf, receiver, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Index { receiver, index } => {
            buf.push_str("(index\n");
            pretty_expr(buf, receiver, depth + 1);
            pretty_expr(buf, index, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Try(inner) => {
            buf.push_str("(try\n");
            pretty_expr(buf, inner, depth + 1);
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            buf.push_str("(if\n");
            pretty_expr(buf, cond, depth + 1);
            pretty_block(buf, then_branch, depth + 1, "then");
            if let Some(eb) = else_branch {
                match eb.as_ref() {
                    ElseBranch::If(e) => pretty_expr(buf, e, depth + 1),
                    ElseBranch::Block(b) => pretty_block(buf, b, depth + 1, "else"),
                }
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Match { scrutinee, arms } => {
            buf.push_str("(match\n");
            pretty_expr(buf, scrutinee, depth + 1);
            for a in arms {
                indent(buf, depth + 1);
                buf.push_str("(arm\n");
                indent(buf, depth + 2);
                buf.push_str("(pattern ");
                pretty_pattern(buf, &a.pattern);
                buf.push_str(")\n");
                if let Some(g) = &a.guard {
                    indent(buf, depth + 2);
                    buf.push_str("(guard\n");
                    pretty_expr(buf, g, depth + 3);
                    indent(buf, depth + 2);
                    buf.push_str(")\n");
                }
                indent(buf, depth + 2);
                buf.push_str("(body\n");
                pretty_expr(buf, &a.body, depth + 3);
                indent(buf, depth + 2);
                buf.push_str(")\n");
                indent(buf, depth + 1);
                buf.push_str(")\n");
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Loop(b) => {
            buf.push_str("(loop\n");
            pretty_block(buf, b, depth + 1, "body");
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::While { cond, body } => {
            buf.push_str("(while\n");
            pretty_expr(buf, cond, depth + 1);
            pretty_block(buf, body, depth + 1, "body");
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::For {
            pattern,
            iter,
            body,
        } => {
            buf.push_str("(for\n");
            indent(buf, depth + 1);
            buf.push_str("(pattern ");
            pretty_pattern(buf, pattern);
            buf.push_str(")\n");
            pretty_expr(buf, iter, depth + 1);
            pretty_block(buf, body, depth + 1, "body");
            indent(buf, depth);
            buf.push_str(")\n");
        }
        ExprKind::Lambda {
            params,
            ret,
            body,
            params_inferred,
        } => {
            write!(buf, "(lambda inferred={}", params_inferred).unwrap();
            buf.push_str(" params=(");
            for (i, p) in params.iter().enumerate() {
                if i > 0 {
                    buf.push(' ');
                }
                match &p.ty {
                    Some(t) => write!(buf, "{}:{}", quote(&p.name), pretty_type(t)).unwrap(),
                    None => write!(buf, "{}", quote(&p.name)).unwrap(),
                }
            }
            buf.push(')');
            if let Some(r) = ret {
                write!(buf, " ret={}", pretty_type(r)).unwrap();
            }
            buf.push('\n');
            match body {
                LambdaBody::Block(b) => pretty_block(buf, b, depth + 1, "body"),
                LambdaBody::Expr(e) => {
                    indent(buf, depth + 1);
                    buf.push_str("(body-expr\n");
                    pretty_expr(buf, e, depth + 2);
                    indent(buf, depth + 1);
                    buf.push_str(")\n");
                }
            }
            indent(buf, depth);
            buf.push_str(")\n");
        }
    }
}

fn pretty_pattern(buf: &mut String, pat: &Pattern) {
    match &pat.kind {
        PatternKind::Wildcard => buf.push('_'),
        PatternKind::Literal(LiteralPattern::Int(n)) => write!(buf, "{}", n).unwrap(),
        PatternKind::Literal(LiteralPattern::Float(v)) => write!(buf, "{}", v).unwrap(),
        PatternKind::Literal(LiteralPattern::Bool(b)) => write!(buf, "{}", b).unwrap(),
        PatternKind::Literal(LiteralPattern::String(s)) => write!(buf, "{}", quote(s)).unwrap(),
        PatternKind::Literal(LiteralPattern::Char(c)) => write!(buf, "{:?}", c).unwrap(),
        PatternKind::Ident(name) => write!(buf, "ident:{}", quote(name)).unwrap(),
        PatternKind::Tuple { name, elements } => {
            if let Some(n) = name {
                write!(buf, "{}(", quote(n)).unwrap();
            } else {
                buf.push('(');
            }
            for (i, e) in elements.iter().enumerate() {
                if i > 0 {
                    buf.push(' ');
                }
                pretty_pattern(buf, e);
            }
            buf.push(')');
        }
        PatternKind::Struct { name, fields } => {
            write!(buf, "{}{{", quote(name)).unwrap();
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    buf.push(' ');
                }
                match &f.pattern {
                    Some(p) => {
                        write!(buf, "{}:", quote(&f.name)).unwrap();
                        pretty_pattern(buf, p);
                    }
                    None => buf.push_str(&quote(&f.name)),
                }
            }
            buf.push('}');
        }
        PatternKind::Range { lo, hi, inclusive } => {
            let sep = if *inclusive { "..=" } else { ".." };
            write!(buf, "{}{}{}", lo, sep, hi).unwrap();
        }
    }
}

fn pretty_type(ty: &Type) -> String {
    match &ty.kind {
        TypeKind::Path(p) => pretty_type_path(p),
        TypeKind::Optional(inner) => format!("{}?", pretty_type(inner)),
        TypeKind::Dyn(p) => format!("dyn {}", pretty_type_path(p)),
        TypeKind::Unit => "()".to_string(),
        TypeKind::Function { params, ret } => {
            let mut s = String::from("fun(");
            for (i, p) in params.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push_str(&pretty_type(p));
            }
            s.push_str(") -> ");
            s.push_str(&pretty_type(ret));
            s
        }
    }
}

fn pretty_type_path(p: &TypePath) -> String {
    let mut out = String::new();
    for (i, seg) in p.segments.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        out.push_str(&seg.name);
        if !seg.generics.is_empty() {
            out.push('<');
            for (j, g) in seg.generics.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                out.push_str(&pretty_type(g));
            }
            out.push('>');
        }
    }
    out
}

fn quote(s: &str) -> String {
    // Escape backslashes and double quotes only. Newlines and tabs
    // are preserved verbatim so they survive the round trip into the
    // golden file.
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn unary_op(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Neg => "neg",
        UnaryOp::Not => "not",
        UnaryOp::Ref => "ref",
    }
}

fn binary_op(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
        BinaryOp::BitAnd => "&",
        BinaryOp::BitOr => "|",
        BinaryOp::BitXor => "^",
        BinaryOp::Shl => "<<",
        BinaryOp::Shr => ">>",
    }
}

fn assign_op(op: AssignOp) -> &'static str {
    match op {
        AssignOp::Assign => "=",
        AssignOp::Add => "+=",
        AssignOp::Sub => "-=",
        AssignOp::Mul => "*=",
        AssignOp::Div => "/=",
        AssignOp::Mod => "%=",
        AssignOp::BitAnd => "&=",
        AssignOp::BitOr => "|=",
        AssignOp::BitXor => "^=",
        AssignOp::Shl => "<<=",
        AssignOp::Shr => ">>=",
    }
}
