//! Stable textual dump of a `MirProgram` for snapshot testing.
//!
//! The format is a flat S-expression style listing of every function,
//! each function's locals table, every basic block's statements, and
//! the terminator. Spans are intentionally omitted (they would cause
//! churn whenever a corpus file is touched).

use std::fmt::Write;

use super::ir::{
    MirBinOp, MirBlock, MirConstant, MirFnRef, MirFunction, MirOperand, MirProgram, MirRvalue,
    MirStatement, MirTerminator, MirUnOp,
};

/// Render the entire `MirProgram` as a multi-line string.
pub fn pretty_program(program: &MirProgram) -> String {
    let mut out = String::new();
    writeln!(&mut out, "(mir").unwrap();
    for f in &program.functions {
        pretty_function(&mut out, f, 1);
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

fn pretty_function(buf: &mut String, f: &MirFunction, depth: usize) {
    indent(buf, depth);
    writeln!(
        buf,
        "(fn {} origin={} ret={}",
        quote(&f.name),
        quote(&f.origin),
        f.ret_ty
    )
    .unwrap();

    indent(buf, depth + 1);
    buf.push_str("(params");
    for p in &f.params {
        let d = f.local_decl(*p);
        write!(buf, " (_{}: {} {})", p.0, d.ty, quote(&d.name)).unwrap();
    }
    buf.push_str(")\n");

    indent(buf, depth + 1);
    writeln!(buf, "(locals").unwrap();
    for (i, d) in f.locals.iter().enumerate() {
        indent(buf, depth + 2);
        writeln!(
            buf,
            "(_{} {} {} param={})",
            i,
            d.ty,
            quote(&d.name),
            d.is_param
        )
        .unwrap();
    }
    indent(buf, depth + 1);
    buf.push_str(")\n");

    indent(buf, depth + 1);
    writeln!(buf, "(entry bb{})", f.entry.0).unwrap();

    for b in &f.blocks {
        pretty_block(buf, b, depth + 1);
    }

    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_block(buf: &mut String, block: &MirBlock, depth: usize) {
    indent(buf, depth);
    writeln!(buf, "(bb{}", block.id.0).unwrap();
    for s in &block.statements {
        pretty_stmt(buf, s, depth + 1);
    }
    pretty_terminator(buf, &block.terminator, depth + 1);
    indent(buf, depth);
    buf.push_str(")\n");
}

fn pretty_stmt(buf: &mut String, s: &MirStatement, depth: usize) {
    indent(buf, depth);
    match s {
        MirStatement::Assign { dst, rvalue } => {
            write!(buf, "(assign _{} ", dst.0).unwrap();
            pretty_rvalue(buf, rvalue);
            buf.push_str(")\n");
        }
        MirStatement::StorageLive(l) => writeln!(buf, "(storage-live _{})", l.0).unwrap(),
        MirStatement::StorageDead(l) => writeln!(buf, "(storage-dead _{})", l.0).unwrap(),
        MirStatement::Nop => writeln!(buf, "(nop)").unwrap(),
    }
}

fn pretty_rvalue(buf: &mut String, r: &MirRvalue) {
    match r {
        MirRvalue::Use(op) => {
            buf.push_str("(use ");
            pretty_operand(buf, op);
            buf.push(')');
        }
        MirRvalue::BinaryOp(op, lhs, rhs) => {
            write!(buf, "(binop {} ", binop_name(*op)).unwrap();
            pretty_operand(buf, lhs);
            buf.push(' ');
            pretty_operand(buf, rhs);
            buf.push(')');
        }
        MirRvalue::UnaryOp(op, x) => {
            write!(buf, "(unop {} ", unop_name(*op)).unwrap();
            pretty_operand(buf, x);
            buf.push(')');
        }
        MirRvalue::Call { callee, args } => {
            buf.push_str("(call ");
            pretty_fnref(buf, callee);
            for a in args {
                buf.push(' ');
                pretty_operand(buf, a);
            }
            buf.push(')');
        }
        MirRvalue::StructCreate { ty, fields, .. } => {
            write!(buf, "(struct-create {}", ty).unwrap();
            for f in fields {
                buf.push(' ');
                pretty_operand(buf, f);
            }
            buf.push(')');
        }
        MirRvalue::EnumCreate {
            ty,
            variant,
            payload,
            ..
        } => {
            write!(buf, "(enum-create {} variant={}", ty, variant).unwrap();
            for p in payload {
                buf.push(' ');
                pretty_operand(buf, p);
            }
            buf.push(')');
        }
        MirRvalue::FieldAccess { base, index } => {
            buf.push_str("(field ");
            pretty_operand(buf, base);
            write!(buf, " #{})", index).unwrap();
        }
        MirRvalue::IndexAccess { base, index } => {
            buf.push_str("(index ");
            pretty_operand(buf, base);
            buf.push(' ');
            pretty_operand(buf, index);
            buf.push(')');
        }
        MirRvalue::ArrayLit { ty, elements } => {
            write!(buf, "(array {}", ty).unwrap();
            for e in elements {
                buf.push(' ');
                pretty_operand(buf, e);
            }
            buf.push(')');
        }
        MirRvalue::Cast { operand, target } => {
            buf.push_str("(cast ");
            pretty_operand(buf, operand);
            write!(buf, " {})", target).unwrap();
        }
        MirRvalue::ClosureCreate { fn_name, captures } => {
            write!(buf, "(closure {}", quote(fn_name)).unwrap();
            for c in captures {
                buf.push(' ');
                pretty_operand(buf, c);
            }
            buf.push(')');
        }
        MirRvalue::DynCoerce {
            value,
            concrete_ty,
            trait_name,
            ..
        } => {
            write!(buf, "(dyn-coerce {} from={} ", trait_name, concrete_ty).unwrap();
            pretty_operand(buf, value);
            buf.push(')');
        }
        MirRvalue::VirtualCall {
            receiver,
            slot,
            args,
            ..
        } => {
            write!(buf, "(vcall slot={} ", slot).unwrap();
            pretty_operand(buf, receiver);
            for a in args {
                buf.push(' ');
                pretty_operand(buf, a);
            }
            buf.push(')');
        }
    }
}

fn pretty_operand(buf: &mut String, op: &MirOperand) {
    match op {
        MirOperand::Copy(l) => write!(buf, "_{}", l.0).unwrap(),
        MirOperand::Const(c) => match c {
            MirConstant::Unit => buf.push_str("()"),
            MirConstant::Bool(b) => write!(buf, "{}", b).unwrap(),
            MirConstant::Int(i) => write!(buf, "{}", i).unwrap(),
            MirConstant::Float(v) => write!(buf, "{}", v).unwrap(),
            MirConstant::Str(s) => buf.push_str(&quote(s)),
            MirConstant::Char(c) => buf.push_str(&quote(&c.to_string())),
            MirConstant::CStr(s) => write!(buf, "c{}", quote(s)).unwrap(),
        },
    }
}

fn pretty_fnref(buf: &mut String, r: &MirFnRef) {
    buf.push_str(&quote(&r.mangled));
}

fn pretty_terminator(buf: &mut String, t: &MirTerminator, depth: usize) {
    indent(buf, depth);
    match t {
        MirTerminator::Goto(b) => writeln!(buf, "(goto bb{})", b.0).unwrap(),
        MirTerminator::SwitchInt {
            discriminant,
            targets,
            otherwise,
        } => {
            buf.push_str("(switch-int ");
            pretty_operand(buf, discriminant);
            buf.push_str(" cases=(");
            for (i, (v, b)) in targets.iter().enumerate() {
                if i > 0 {
                    buf.push(' ');
                }
                write!(buf, "{}:bb{}", v, b.0).unwrap();
            }
            writeln!(buf, ") otherwise=bb{})", otherwise.0).unwrap();
        }
        MirTerminator::SwitchEnum {
            discriminant,
            targets,
            otherwise,
        } => {
            buf.push_str("(switch-enum ");
            pretty_operand(buf, discriminant);
            buf.push_str(" cases=(");
            for (i, (v, b)) in targets.iter().enumerate() {
                if i > 0 {
                    buf.push(' ');
                }
                write!(buf, "{}:bb{}", v, b.0).unwrap();
            }
            buf.push(')');
            if let Some(o) = otherwise {
                write!(buf, " otherwise=bb{}", o.0).unwrap();
            }
            buf.push_str(")\n");
        }
        MirTerminator::Return(op) => {
            buf.push_str("(return ");
            pretty_operand(buf, op);
            buf.push_str(")\n");
        }
        MirTerminator::Unreachable => buf.push_str("(unreachable)\n"),
    }
}

fn binop_name(op: MirBinOp) -> &'static str {
    match op {
        MirBinOp::Add => "+",
        MirBinOp::Sub => "-",
        MirBinOp::Mul => "*",
        MirBinOp::Div => "/",
        MirBinOp::Mod => "%",
        MirBinOp::Eq => "==",
        MirBinOp::Ne => "!=",
        MirBinOp::Lt => "<",
        MirBinOp::Le => "<=",
        MirBinOp::Gt => ">",
        MirBinOp::Ge => ">=",
        MirBinOp::And => "&&",
        MirBinOp::Or => "||",
        MirBinOp::BitAnd => "&",
        MirBinOp::BitOr => "|",
        MirBinOp::BitXor => "^",
        MirBinOp::Shl => "<<",
        MirBinOp::Shr => ">>",
    }
}

fn unop_name(op: MirUnOp) -> &'static str {
    match op {
        MirUnOp::Neg => "neg",
        MirUnOp::Not => "not",
        MirUnOp::Ref => "ref",
    }
}
