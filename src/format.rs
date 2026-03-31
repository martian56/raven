//! Pretty-print Raven source (parse and re-emit with consistent style).

use crate::ast::{ASTNode, Expression, Operator};
use crate::error::RavenError;
use crate::lexer::Lexer;
use crate::parser::Parser;

const INDENT: &str = "    ";

/// Format a Raven source string. `filename` is used only for parse error messages.
pub fn format_source(source: &str, filename: &str) -> Result<String, RavenError> {
    let lexer = Lexer::new(source.to_string());
    let mut parser = Parser::new(lexer, source.to_string());
    let ast = parser
        .parse()
        .map_err(|e| e.with_filename(filename.to_string()))?;
    Ok(format_ast(&ast))
}

fn format_ast(ast: &ASTNode) -> String {
    let mut out = match ast {
        ASTNode::Block(stmts) => stmts
            .iter()
            .map(|s| format_stmt(s, 0))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => format_stmt(ast, 0),
    };
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn format_stmt(node: &ASTNode, indent: usize) -> String {
    let pad = INDENT.repeat(indent);
    match node {
        ASTNode::Block(stmts) => stmts
            .iter()
            .map(|s| format_stmt(s, indent))
            .collect::<Vec<_>>()
            .join("\n"),
        ASTNode::VariableDecl(name, expr) => {
            format!("{}let {} = {};", pad, name, format_expr(expr))
        }
        ASTNode::VariableDeclTyped(name, ty, expr) => {
            if matches!(expr.as_ref(), Expression::Uninitialized) {
                format!("{}let {}: {};", pad, name, ty)
            } else {
                format!("{}let {}: {} = {};", pad, name, ty, format_expr(expr))
            }
        }
        ASTNode::FunctionDecl(name, ret, params, body) => {
            let params_s = params
                .iter()
                .map(|p| format!("{}: {}", p.name, p.param_type))
                .collect::<Vec<_>>()
                .join(", ");
            let body_s = format_block_body(body, indent);
            format!(
                "{}fun {}({}) -> {} {{\n{}\n{}}}",
                pad, name, params_s, ret, body_s, pad
            )
        }
        ASTNode::StructDecl(name, fields) => {
            let lines: Vec<String> = fields
                .iter()
                .map(|f| format!("{}{}: {},", INDENT.repeat(indent + 1), f.name, f.field_type))
                .collect();
            format!(
                "{}struct {} {{\n{}\n{}}}",
                pad,
                name,
                lines.join("\n"),
                pad
            )
        }
        ASTNode::ImplBlock(struct_name, methods) => {
            let mut parts = Vec::new();
            for (mname, ret, params, body) in methods {
                let params_s = params
                    .iter()
                    .map(|p| format!("{}: {}", p.name, p.param_type))
                    .collect::<Vec<_>>()
                    .join(", ");
                let body_s = format_block_body(body, indent + 1);
                parts.push(format!(
                    "{}fun {}({}) -> {} {{\n{}\n{}}}",
                    INDENT.repeat(indent + 1),
                    mname,
                    params_s,
                    ret,
                    body_s,
                    INDENT.repeat(indent + 1)
                ));
            }
            format!(
                "{}impl {} {{\n{}\n{}}}",
                pad,
                struct_name,
                parts.join("\n\n"),
                pad
            )
        }
        ASTNode::EnumDecl(name, variants) => {
            let lines: Vec<String> = variants
                .iter()
                .map(|v| format!("{}{},", INDENT.repeat(indent + 1), v))
                .collect();
            format!(
                "{}enum {} {{\n{}\n{}}}",
                pad,
                name,
                lines.join("\n"),
                pad
            )
        }
        ASTNode::ForLoop(init, cond, inc, body) => {
            let init_s = format_stmt_strip_pad(init, 0);
            let cond_s = format_expr(cond);
            let inc_s = match inc.as_ref() {
                ASTNode::Assignment(lhs, rhs) => {
                    format!("{} = {}", format_expr(lhs), format_expr(rhs))
                }
                _ => format_stmt_strip_pad(inc, 0),
            };
            let body_s = format_block_body(body, indent);
            format!(
                "{}for ({}; {}; {}) {{\n{}\n{}}}",
                pad, init_s, cond_s, inc_s, body_s, pad
            )
        }
        ASTNode::WhileLoop(cond, body) => {
            let body_s = format_block_body(body, indent);
            format!(
                "{}while ({}) {{\n{}\n{}}}",
                pad,
                format_expr(cond),
                body_s,
                pad
            )
        }
        ASTNode::Assignment(lhs, rhs) => {
            format!("{}{} = {};", pad, format_expr(lhs), format_expr(rhs))
        }
        ASTNode::IfStatement(cond, then_b, else_if, else_b) => {
            let mut s = format!(
                "{}if ({}) {{\n{}\n{}}}",
                pad,
                format_expr(cond),
                format_block_body(then_b, indent),
                pad
            );
            if let Some(elif) = else_if {
                s.push_str(&format_else_if_chain(elif, indent));
            }
            if let Some(else_block) = else_b {
                s.push_str(&format!(
                    " else {{\n{}\n{}}}",
                    format_block_body(else_block, indent),
                    pad
                ));
            }
            s
        }
        ASTNode::Print(e) => format!("{}print({});", pad, format_expr(e)),
        ASTNode::FunctionCall(name, args) => {
            format!(
                "{}{}({});",
                pad,
                name,
                format_expr_list(args)
            )
        }
        ASTNode::MethodCall(obj, name, args) => {
            format!(
                "{}{}.{}({});",
                pad,
                format_expr(obj),
                name,
                format_expr_list(args)
            )
        }
        ASTNode::ExpressionStatement(e) => format!("{}{};", pad, format_expr(e)),
        ASTNode::Return(e) => format!("{}return {};", pad, format_expr(e)),
        ASTNode::Import(module, alias) => match alias {
            Some(a) => format!("{}import {} from \"{}\";", pad, a, module),
            None => format!("{}import \"{}\";", pad, module),
        },
        ASTNode::ImportSelective(module, items) => {
            let list = items.join(", ");
            format!("{}import {{ {} }} from \"{}\";", pad, list, module)
        }
        ASTNode::Export(inner) => {
            let inner = format_stmt(inner, indent);
            if let Some(rest) = inner.strip_prefix(&pad) {
                format!("{}export {}", pad, rest)
            } else {
                format!("{}export {}", pad, inner.trim_start())
            }
        }
    }
}

/// Format a single-statement node without outer indentation (for `for` init / export).
fn format_stmt_strip_pad(node: &ASTNode, base_indent: usize) -> String {
    let s = format_stmt(node, base_indent);
    s.trim_start().to_string()
}

fn format_else_if_chain(node: &ASTNode, indent: usize) -> String {
    let pad = INDENT.repeat(indent);
    match node {
        ASTNode::IfStatement(cond, then_b, else_if, else_b) => {
            let mut s = format!(
                " elseif ({}) {{\n{}\n{}}}",
                format_expr(cond),
                format_block_body(then_b, indent),
                pad
            );
            if let Some(elif) = else_if {
                s.push_str(&format_else_if_chain(elif, indent));
            }
            if let Some(else_block) = else_b {
                s.push_str(&format!(
                    " else {{\n{}\n{}}}",
                    format_block_body(else_block, indent),
                    pad
                ));
            }
            s
        }
        _ => String::new(),
    }
}

fn format_block_body(body: &ASTNode, outer_indent: usize) -> String {
    match body {
        ASTNode::Block(stmts) => stmts
            .iter()
            .map(|s| format_stmt(s, outer_indent + 1))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => format_stmt(body, outer_indent + 1),
    }
}

fn format_expr_list(args: &[Expression]) -> String {
    args.iter()
        .map(format_expr)
        .collect::<Vec<_>>()
        .join(", ")
}

fn precedence(op: &Operator) -> u8 {
    match op {
        Operator::Or => 1,
        Operator::And => 2,
        Operator::Equal | Operator::NotEqual => 3,
        Operator::LessThan
        | Operator::GreaterThan
        | Operator::LessEqual
        | Operator::GreaterEqual => 4,
        Operator::Add | Operator::Subtract => 5,
        Operator::Multiply | Operator::Divide | Operator::Modulo => 6,
        Operator::UnaryMinus | Operator::Not => 7,
    }
}

fn op_str(op: &Operator) -> &'static str {
    match op {
        Operator::UnaryMinus => "-",
        Operator::Not => "not",
        Operator::Add => "+",
        Operator::Subtract => "-",
        Operator::Multiply => "*",
        Operator::Divide => "/",
        Operator::Modulo => "%",
        Operator::Equal => "==",
        Operator::NotEqual => "!=",
        Operator::LessThan => "<",
        Operator::GreaterThan => ">",
        Operator::LessEqual => "<=",
        Operator::GreaterEqual => ">=",
        Operator::And => "and",
        Operator::Or => "or",
    }
}

fn format_expr(e: &Expression) -> String {
    format_expr_ctx(e, 0, true)
}

fn format_expr_ctx(expr: &Expression, parent_prec: u8, is_left: bool) -> String {
    match expr {
        Expression::BinaryOp(l, op, r) => {
            let p = precedence(op);
            let left_s = format_expr_ctx(l, p, true);
            let right_s = format_expr_ctx(r, p, false);
            let inner = format!("{} {} {}", left_s, op_str(op), right_s);
            if p < parent_prec || (p == parent_prec && !is_left) {
                format!("({})", inner)
            } else {
                inner
            }
        }
        Expression::UnaryOp(op, e) => {
            let operand = format_unary_operand(e);
            match op {
                Operator::UnaryMinus => format!("-{}", operand),
                Operator::Not => format!("not {}", operand),
                _ => format!("{} {}", op_str(op), operand),
            }
        }
        Expression::Integer(i) => format!("{}", i),
        Expression::Float(f) => format_float(*f),
        Expression::Boolean(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Expression::StringLiteral(s) => format!("\"{}\"", escape_string(s)),
        Expression::Identifier(s) => s.clone(),
        Expression::FunctionCall(name, args) => format!("{}({})", name, format_expr_list(args)),
        Expression::ArrayLiteral(el) => {
            format!("[{}]", format_expr_list(el))
        }
        Expression::ArrayIndex(a, i) => {
            let base = match a.as_ref() {
                Expression::BinaryOp(..) => format!("({})", format_expr(a)),
                _ => format_expr_ctx(a, 0, true),
            };
            format!("{}[{}]", base, format_expr(i))
        }
        Expression::MethodCall(obj, name, args) => {
            format!(
                "{}.{}({})",
                format_expr_ctx(obj, 0, true),
                name,
                format_expr_list(args)
            )
        }
        Expression::StructInstantiation(name, fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(n, e)| format!("{}: {}", n, format_expr(e)))
                .collect();
            format!("{} {{ {} }}", name, parts.join(", "))
        }
        Expression::FieldAccess(obj, field) => {
            format!("{}.{}", format_expr_ctx(obj, 0, true), field)
        }
        Expression::EnumVariant(enum_name, variant) => {
            format!("{}::{}", enum_name, variant)
        }
        Expression::Uninitialized => "<uninitialized>".to_string(),
    }
}

fn format_unary_operand(e: &Expression) -> String {
    match e {
        Expression::BinaryOp(..) => format!("({})", format_expr(e)),
        _ => format_expr(e),
    }
}

fn format_float(f: f64) -> String {
    if f.is_nan() {
        return "nan".to_string();
    }
    if f.is_infinite() {
        return if f.is_sign_positive() {
            "inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    if f == 0.0 && f.is_sign_negative() {
        return "-0.0".to_string();
    }
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}.0", f as i64)
    } else {
        format!("{}", f)
    }
}

fn escape_string(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_simple() {
        let src = "fun main() -> void {\n    print(\"hi\");\n}\n\nmain();\n";
        let out = format_source(src, "t.rv").unwrap();
        let _ = format_source(&out, "t2.rv").unwrap();
    }

    #[test]
    fn preserves_binary_precedence() {
        let src = "let x: int = 1 + 2 * 3;\n";
        let out = format_source(src, "t.rv").unwrap();
        assert!(out.contains("1 + 2 * 3"));
    }
}
