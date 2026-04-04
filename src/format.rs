//! Pretty-print Raven source (parse and re-emit with consistent style).
//!
//! Goals: readable vertical rhythm (blank lines between imports and declarations, between
//! top-level items, and before control flow inside blocks), wrap long signatures and literals,
//! and stable `elseif` / `else` layout.

use crate::ast::{ASTNode, EnumMember, Expression, ImplMember, Operator, Parameter, StructMember};
use crate::error::RavenError;
use crate::lexer::Lexer;
use crate::parser::Parser;
use std::fs;
use std::path::Path;

/// Options for [`format_source_with_options`], including values from `rv.toml` under `[fmt]`.
///
/// - `indent_width` (default 4): spaces per indent level; TOML values are clamped to 1–16.
/// - `wrap_width` (default 88): soft line length for wrapping; TOML values are clamped to 40–200.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatOptions {
    /// Spaces per indent level (default `4`).
    pub indent_width: usize,
    /// Target width for wrapping long lines (default `88`, clamped when read from TOML).
    pub wrap_width: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            indent_width: 4,
            wrap_width: 88,
        }
    }
}

impl FormatOptions {
    fn pad(&self, depth: usize) -> String {
        let w = self.indent_width.max(1);
        " ".repeat(w.saturating_mul(depth))
    }

    fn unit(&self) -> String {
        " ".repeat(self.indent_width.max(1))
    }

    /// Read `[fmt]` from `rv.toml` at `path`. Missing file or invalid values fall back to defaults.
    pub fn from_rv_toml(path: &Path) -> Self {
        Self::from_rv_toml_inner(path).unwrap_or_default()
    }

    fn from_rv_toml_inner(path: &Path) -> Option<Self> {
        let s = fs::read_to_string(path).ok()?;
        let v: toml::Value = toml::from_str(&s).ok()?;
        let mut o = Self::default();
        let fmt = v.get("fmt")?.as_table()?;
        if let Some(w) = fmt.get("wrap_width").and_then(|x| x.as_integer()) {
            let w = usize::try_from(w).ok()?;
            if (40..=200).contains(&w) {
                o.wrap_width = w;
            }
        }
        if let Some(i) = fmt.get("indent_width").and_then(|x| x.as_integer()) {
            let i = usize::try_from(i).ok()?;
            if (1..=16).contains(&i) {
                o.indent_width = i;
            }
        }
        Some(o)
    }
}

/// Format a Raven source string. `filename` is used only for parse error messages.
pub fn format_source(source: &str, filename: &str) -> Result<String, RavenError> {
    format_source_with_options(source, filename, &FormatOptions::default())
}

/// Format with explicit options (e.g. from `rv.toml` `[fmt]` via [`FormatOptions::from_rv_toml`]).
pub fn format_source_with_options(
    source: &str,
    filename: &str,
    opts: &FormatOptions,
) -> Result<String, RavenError> {
    let lexer = Lexer::new(source.to_string());
    let mut parser = Parser::new(lexer, source.to_string());
    let ast = parser
        .parse()
        .map_err(|e| e.with_filename(filename.to_string()))?;
    Ok(format_ast(&ast, opts))
}

fn format_ast(ast: &ASTNode, opts: &FormatOptions) -> String {
    let mut out = match ast {
        ASTNode::Block(stmts) => join_block_stmts(stmts, 0, true, opts),
        _ => format_stmt(ast, 0, opts),
    };
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn is_import(node: &ASTNode) -> bool {
    matches!(node, ASTNode::Import(..) | ASTNode::ImportSelective(..))
}

fn is_major_decl(node: &ASTNode) -> bool {
    match node {
        ASTNode::FunctionDecl(..)
        | ASTNode::StructDecl(..)
        | ASTNode::ImplBlock(..)
        | ASTNode::EnumDecl(..) => true,
        ASTNode::Export(inner) => is_major_decl(inner.as_ref()),
        _ => false,
    }
}

/// Blank line between consecutive top-level items for visual sections (imports vs code, decl vs decl).
fn blank_between_top_level(prev: &ASTNode, next: &ASTNode) -> bool {
    if matches!(next, ASTNode::Comment(_)) {
        return false;
    }
    if matches!(prev, ASTNode::Comment(_)) {
        return matches!(
            next,
            ASTNode::FunctionDecl(..)
                | ASTNode::StructDecl(..)
                | ASTNode::ImplBlock(..)
                | ASTNode::EnumDecl(..)
                | ASTNode::Export(..)
        ) || is_import(next);
    }
    if is_import(prev) && !is_import(next) {
        return true;
    }
    is_major_decl(prev) && is_major_decl(next)
}

/// Blank line before control-flow statements inside blocks (not before the first statement).
fn blank_between_inner(prev: &ASTNode, next: &ASTNode) -> bool {
    if matches!(prev, ASTNode::Comment(_)) || matches!(next, ASTNode::Comment(_)) {
        return false;
    }
    matches!(
        next,
        ASTNode::IfStatement(..) | ASTNode::WhileLoop(..) | ASTNode::ForLoop(..)
    )
}

/// Single-line or wrapped parameter lists for `fun` / `impl` signatures.
fn format_parameter_list(params: &[Parameter], indent: usize, opts: &FormatOptions) -> String {
    let parts: Vec<String> = params
        .iter()
        .map(|p| format!("{}: {}", p.name, p.param_type))
        .collect();
    let single_line = parts.join(", ");
    if params.len() <= 3 && single_line.len() + 16 <= opts.wrap_width {
        return single_line;
    }
    let inner_pad = opts.pad(indent + 1);
    let close_pad = opts.pad(indent);
    let inner = params
        .iter()
        .map(|p| format!("{}{}: {}", inner_pad, p.name, p.param_type))
        .collect::<Vec<_>>()
        .join(",\n");
    format!("\n{}\n{}", inner, close_pad)
}

fn join_block_stmts(
    stmts: &[ASTNode],
    indent: usize,
    top_level: bool,
    opts: &FormatOptions,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (i, s) in stmts.iter().enumerate() {
        if i > 0 {
            let prev = &stmts[i - 1];
            let blank = if top_level {
                blank_between_top_level(prev, s)
            } else {
                blank_between_inner(prev, s)
            };
            if blank {
                parts.push(String::new());
            }
        }
        parts.push(format_stmt(s, indent, opts));
    }
    parts.join("\n")
}

fn format_stmt(node: &ASTNode, indent: usize, opts: &FormatOptions) -> String {
    let pad = opts.pad(indent);
    match node {
        ASTNode::Block(stmts) => join_block_stmts(stmts, indent, false, opts),
        ASTNode::VariableDecl(name, expr) => {
            format!("{}let {} = {};", pad, name, format_expr(expr, opts))
        }
        ASTNode::VariableDeclTyped(name, ty, expr) => {
            if matches!(expr.as_ref(), Expression::Uninitialized) {
                format!("{}let {}: {};", pad, name, ty)
            } else {
                format!("{}let {}: {} = {};", pad, name, ty, format_expr(expr, opts))
            }
        }
        ASTNode::FunctionDecl(name, ret, params, body) => {
            let params_s = format_parameter_list(params, indent, opts);
            let body_s = format_block_body(body, indent, opts);
            format!(
                "{}fun {}({}) -> {} {{\n{}\n{}}}",
                pad, name, params_s, ret, body_s, pad
            )
        }
        ASTNode::StructDecl(name, members) => {
            let lines: Vec<String> = members
                .iter()
                .map(|m| match m {
                    StructMember::Field(f) => {
                        format!("{}{}: {},", opts.pad(indent + 1), f.name, f.field_type)
                    }
                    StructMember::Comment(text) => {
                        format!("{}{}", opts.pad(indent + 1), text)
                    }
                })
                .collect();
            format!("{}struct {} {{\n{}\n{}}}", pad, name, lines.join("\n"), pad)
        }
        ASTNode::ImplBlock(struct_name, methods) => {
            let mut parts = Vec::new();
            for m in methods {
                match m {
                    ImplMember::Method(mname, ret, params, body) => {
                        let params_s = format_parameter_list(params, indent + 1, opts);
                        let body_s = format_block_body(body, indent + 1, opts);
                        parts.push(format!(
                            "{}fun {}({}) -> {} {{\n{}\n{}}}",
                            opts.pad(indent + 1),
                            mname,
                            params_s,
                            ret,
                            body_s,
                            opts.pad(indent + 1)
                        ));
                    }
                    ImplMember::Comment(text) => {
                        parts.push(format!("{}{}", opts.pad(indent + 1), text));
                    }
                }
            }
            format!(
                "{}impl {} {{\n{}\n{}}}",
                pad,
                struct_name,
                parts.join("\n\n"),
                pad
            )
        }
        ASTNode::EnumDecl(name, members) => {
            let lines: Vec<String> = members
                .iter()
                .map(|m| match m {
                    EnumMember::Variant(v) => format!("{}{},", opts.pad(indent + 1), v),
                    EnumMember::Comment(text) => format!("{}{}", opts.pad(indent + 1), text),
                })
                .collect();
            format!("{}enum {} {{\n{}\n{}}}", pad, name, lines.join("\n"), pad)
        }
        ASTNode::Comment(text) => format!("{}{}", pad, text),
        ASTNode::ForLoop(init, cond, inc, body) => {
            let init_s = format_stmt_strip_pad(init, 0, opts);
            let cond_s = format_expr(cond, opts);
            let inc_s = match inc.as_ref() {
                ASTNode::Assignment(lhs, rhs) => {
                    format!("{} = {}", format_expr(lhs, opts), format_expr(rhs, opts))
                }
                _ => format_stmt_strip_pad(inc, 0, opts),
            };
            let body_s = format_block_body(body, indent, opts);
            format!(
                "{}for ({}; {}; {}) {{\n{}\n{}}}",
                pad, init_s, cond_s, inc_s, body_s, pad
            )
        }
        ASTNode::WhileLoop(cond, body) => {
            let body_s = format_block_body(body, indent, opts);
            format!(
                "{}while ({}) {{\n{}\n{}}}",
                pad,
                format_expr(cond, opts),
                body_s,
                pad
            )
        }
        ASTNode::Assignment(lhs, rhs) => {
            format!(
                "{}{} = {};",
                pad,
                format_expr(lhs, opts),
                format_expr(rhs, opts)
            )
        }
        ASTNode::IfStatement(cond, then_b, else_if, else_b) => {
            let mut s = format!(
                "{}if ({}) {{\n{}\n{}}}",
                pad,
                format_expr(cond, opts),
                format_block_body(then_b, indent, opts),
                pad
            );
            if let Some(elif) = else_if {
                s.push_str(&format_else_if_chain(elif, indent, opts));
            }
            if let Some(else_block) = else_b {
                s.push_str(&format!(
                    "\n{} else {{\n{}\n{}}}",
                    pad,
                    format_block_body(else_block, indent, opts),
                    pad
                ));
            }
            s
        }
        ASTNode::Print(e) => format!("{}print({});", pad, format_expr(e, opts)),
        ASTNode::FunctionCall(name, args) => {
            format!("{}{}({});", pad, name, format_expr_list(args, opts))
        }
        ASTNode::MethodCall(obj, name, args) => {
            format!(
                "{}{}.{}({});",
                pad,
                format_expr(obj, opts),
                name,
                format_expr_list(args, opts)
            )
        }
        ASTNode::ExpressionStatement(e) => format!("{}{};", pad, format_expr(e, opts)),
        ASTNode::Return(e) => format!("{}return {};", pad, format_expr(e, opts)),
        ASTNode::Import(module, alias) => match alias {
            Some(a) => format!("{}import {} from \"{}\";", pad, a, module),
            None => format!("{}import \"{}\";", pad, module),
        },
        ASTNode::ImportSelective(module, items) => {
            let list = items.join(", ");
            let one_line = format!("{}import {{ {} }} from \"{}\";", pad, list, module);
            if one_line.len() <= opts.wrap_width || items.len() <= 3 {
                return one_line;
            }
            let inner_pad = opts.pad(indent + 1);
            let inner = items
                .iter()
                .map(|n| format!("{}{}", inner_pad, n))
                .collect::<Vec<_>>()
                .join(",\n");
            format!(
                "{}import {{\n{}\n{}}} from \"{}\";",
                pad,
                inner,
                opts.pad(indent),
                module
            )
        }
        ASTNode::Export(inner) => {
            let inner = format_stmt(inner, indent, opts);
            if let Some(rest) = inner.strip_prefix(&pad) {
                format!("{}export {}", pad, rest)
            } else {
                format!("{}export {}", pad, inner.trim_start())
            }
        }
    }
}

/// Format a single-statement node without outer indentation (for `for` init / export).
fn format_stmt_strip_pad(node: &ASTNode, base_indent: usize, opts: &FormatOptions) -> String {
    let s = format_stmt(node, base_indent, opts);
    s.trim_start().to_string()
}

fn format_else_if_chain(node: &ASTNode, indent: usize, opts: &FormatOptions) -> String {
    let pad = opts.pad(indent);
    match node {
        ASTNode::IfStatement(cond, then_b, else_if, else_b) => {
            let mut s = format!(
                "\n{}elseif ({}) {{\n{}\n{}}}",
                pad,
                format_expr(cond, opts),
                format_block_body(then_b, indent, opts),
                pad
            );
            if let Some(elif) = else_if {
                s.push_str(&format_else_if_chain(elif, indent, opts));
            }
            if let Some(else_block) = else_b {
                s.push_str(&format!(
                    "\n{} else {{\n{}\n{}}}",
                    pad,
                    format_block_body(else_block, indent, opts),
                    pad
                ));
            }
            s
        }
        _ => String::new(),
    }
}

fn format_block_body(body: &ASTNode, outer_indent: usize, opts: &FormatOptions) -> String {
    match body {
        ASTNode::Block(stmts) => join_block_stmts(stmts, outer_indent + 1, false, opts),
        _ => format_stmt(body, outer_indent + 1, opts),
    }
}

fn format_expr_list(args: &[Expression], opts: &FormatOptions) -> String {
    args.iter()
        .map(|e| format_expr(e, opts))
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

fn format_expr(e: &Expression, opts: &FormatOptions) -> String {
    format_expr_ctx(e, 0, true, opts)
}

fn format_expr_ctx(
    expr: &Expression,
    parent_prec: u8,
    is_left: bool,
    opts: &FormatOptions,
) -> String {
    match expr {
        Expression::BinaryOp(l, op, r) => {
            let p = precedence(op);
            let left_s = format_expr_ctx(l, p, true, opts);
            let right_s = format_expr_ctx(r, p, false, opts);
            let inner = format!("{} {} {}", left_s, op_str(op), right_s);
            if p < parent_prec || (p == parent_prec && !is_left) {
                format!("({})", inner)
            } else {
                inner
            }
        }
        Expression::UnaryOp(op, e) => {
            let operand = format_unary_operand(e, opts);
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
        Expression::FunctionCall(name, args) => {
            let compact = format!("{}({})", name, format_expr_list(args, opts));
            if args.len() <= 3 && compact.len() <= opts.wrap_width {
                return compact;
            }
            let u = opts.unit();
            let inner = args
                .iter()
                .map(|a| format!("{}{}", u, format_expr(a, opts)))
                .collect::<Vec<_>>()
                .join(",\n");
            format!("{}(\n{}\n)", name, inner)
        }
        Expression::ArrayLiteral(el) => {
            let compact = format!("[{}]", format_expr_list(el, opts));
            if el.len() <= 4 && compact.len() <= opts.wrap_width {
                return compact;
            }
            let u = opts.unit();
            let inner = el
                .iter()
                .map(|e| format!("{}{}", u, format_expr(e, opts)))
                .collect::<Vec<_>>()
                .join(",\n");
            format!("[\n{}\n]", inner)
        }
        Expression::ArrayIndex(a, i) => {
            let base = match a.as_ref() {
                Expression::BinaryOp(..) => format!("({})", format_expr(a, opts)),
                _ => format_expr_ctx(a, 0, true, opts),
            };
            format!("{}[{}]", base, format_expr(i, opts))
        }
        Expression::MethodCall(obj, name, args) => {
            let obj_s = format_expr_ctx(obj, 0, true, opts);
            let compact = format!("{}.{}({})", obj_s, name, format_expr_list(args, opts));
            if args.len() <= 3 && compact.len() <= opts.wrap_width {
                return compact;
            }
            let u = opts.unit();
            let inner = args
                .iter()
                .map(|a| format!("{}{}", u, format_expr(a, opts)))
                .collect::<Vec<_>>()
                .join(",\n");
            format!("{}.{}(\n{}\n)", obj_s, name, inner)
        }
        Expression::StructInstantiation(name, fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(n, e)| format!("{}: {}", n, format_expr(e, opts)))
                .collect();
            let joined = parts.join(", ");
            let compact = format!("{} {{ {} }}", name, joined);
            if fields.len() <= 2 && compact.len() <= opts.wrap_width {
                return compact;
            }
            let u = opts.unit();
            let inner = fields
                .iter()
                .map(|(n, e)| format!("{}{}: {}", u, n, format_expr(e, opts)))
                .collect::<Vec<_>>()
                .join(",\n");
            format!("{} {{\n{}\n}}", name, inner)
        }
        Expression::FieldAccess(obj, field) => {
            format!("{}.{}", format_expr_ctx(obj, 0, true, opts), field)
        }
        Expression::EnumVariant(enum_name, variant) => {
            format!("{}::{}", enum_name, variant)
        }
        Expression::Uninitialized => "<uninitialized>".to_string(),
    }
}

fn format_unary_operand(e: &Expression, opts: &FormatOptions) -> String {
    match e {
        Expression::BinaryOp(..) => format!("({})", format_expr(e, opts)),
        _ => format_expr(e, opts),
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
            '\0' => out.push_str("\\0"),
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

    #[test]
    fn preserves_line_comments_when_formatting() {
        let src = "// header\nfun main() -> void {\n    print(\"hi\");\n}\n";
        let out = format_source(src, "t.rv").unwrap();
        assert!(out.contains("// header"));
        assert!(out.contains("fun main()"));
    }

    #[test]
    fn ignores_comment_markers_inside_strings() {
        let src = "let s: string = \"http://x\";\n";
        let out = format_source(src, "t.rv").unwrap();
        assert!(out.contains("http://x"));
    }

    #[test]
    fn blank_line_after_import_group_before_fun() {
        let src = "import \"a\";\nimport \"b\";\nfun main() -> void {\n    print(1);\n}\n";
        let out = format_source(src, "t.rv").unwrap();
        assert!(
            out.contains("import \"b\";\n\nfun main"),
            "expected blank line between imports and fun, got:\n{out:?}"
        );
    }

    #[test]
    fn blank_line_between_top_level_functions() {
        let src = "fun a() -> void {\n}\nfun b() -> void {\n}\n";
        let out = format_source(src, "t.rv").unwrap();
        assert!(out.contains("}\n\nfun b"));
    }

    #[test]
    fn wraps_long_parameter_list() {
        let src = "fun f(a: int, b: int, c: int, d: int) -> void {\n}\n";
        let out = format_source(src, "t.rv").unwrap();
        assert!(out.contains('\n') && out.contains("a: int") && out.contains("d: int"));
    }

    #[test]
    fn narrow_wrap_width_wraps_earlier() {
        let src = "fun f(a: int, b: int, c: int, d: int) -> void {\n}\n";
        let opts = FormatOptions {
            indent_width: 4,
            wrap_width: 40,
        };
        let out = format_source_with_options(src, "t.rv", &opts).unwrap();
        assert!(out.contains("a: int") && out.contains('\n'));
    }

    #[test]
    fn fmt_options_from_rv_toml_reads_keys() {
        let tmp = std::env::temp_dir().join("raven_test_fmt_rv.toml");
        std::fs::write(
            &tmp,
            "[package]\nname = \"x\"\n\n[fmt]\nwrap_width = 50\nindent_width = 2\n",
        )
        .unwrap();
        let o = FormatOptions::from_rv_toml(&tmp);
        assert_eq!(o.wrap_width, 50);
        assert_eq!(o.indent_width, 2);
        let _ = std::fs::remove_file(&tmp);
    }
}
