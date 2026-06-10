//! Canonical source formatter for the Raven v2 grammar.
//!
//! There are no style options. Given any parseable source, the formatter
//! emits a single canonical rendering that re-parses to an equivalent AST
//! and is idempotent: formatting an already-formatted file is a no-op.
//!
//! The pipeline lexes and parses the input, then walks the AST emitting
//! canonical text. Line comments are recovered from the source separately
//! (the lexer drops them) and woven back in by source position. See
//! `docs/v2/specs/formatter.md` for the full set of rules.

use std::fmt::Write as _;

use crate::ast::{
    AssignOp, BinaryOp, Block, Const, Decl, DeclKind, ElseBranch, Enum, Expr, ExprKind, Extern,
    ExternFn, File, Function, FunctionBody, GenericParam, Impl, Import, ImportSource, LambdaBody,
    LambdaParam, LetDecl, LiteralPattern, MacroDef, MacroDelim, MatchArm, Param, Pattern,
    PatternKind, Stmt, StmtKind, StrFragment, Struct, StructField, Trait, Type, TypeKind, TypePath,
    UnaryOp, VariantPayload,
};
use crate::lexer::{Lexer, Token, TokenKind};
use crate::parser::parse;

mod comments;

use comments::Comment;

/// The indent width used when no manifest `[fmt].indent_width` applies.
const DEFAULT_INDENT_WIDTH: u32 = 4;

/// A formatting failure. The only way to fail is to be handed source that
/// does not lex or parse: the formatter cannot canonicalize what it cannot
/// understand.
#[derive(Debug)]
pub enum FormatError {
    /// The source failed to lex or parse. The inner message is the
    /// underlying compiler diagnostic.
    Parse(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::Parse(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for FormatError {}

/// Format Raven source text into its canonical form.
pub fn format_source(src: &str) -> Result<String, FormatError> {
    format_source_with(src, DEFAULT_INDENT_WIDTH)
}

/// Format `src` using `indent_width` spaces per level. `rvpm fmt` passes the
/// width from the manifest's `[fmt].indent_width`.
pub fn format_source_with(src: &str, indent_width: u32) -> Result<String, FormatError> {
    let tokens = Lexer::new(src, "<fmt>")
        .tokenize()
        .map_err(|e| FormatError::Parse(e.to_string()))?;
    // Macro definitions (`macro name { ... }`) and invocations (`name!(...)`)
    // are parsed into dedicated AST nodes here (the formatter parses
    // un-expanded source, unlike the compile pipeline, which expands macros
    // at the token level first) and rendered by `macro_decl` / the
    // `MacroCall` expression arm.
    let file = parse(&tokens).map_err(|e| FormatError::Parse(e.to_string()))?;
    let comments = comments::scan(src);
    let indent_unit = " ".repeat(indent_width as usize);
    let mut p = Printer::new(src, comments, indent_unit);
    p.file(&file);
    Ok(p.finish())
}

/// Accumulates the canonical output and tracks indentation, the current
/// source line, and the pending comment cursor.
struct Printer<'a> {
    out: String,
    indent: usize,
    /// The string emitted for one indent level (`indent_width` spaces).
    indent_unit: String,
    src: &'a str,
    comments: Vec<Comment>,
    /// Index of the next comment not yet emitted.
    next_comment: usize,
}

impl<'a> Printer<'a> {
    fn new(src: &'a str, comments: Vec<Comment>, indent_unit: String) -> Self {
        Printer {
            out: String::new(),
            indent: 0,
            indent_unit,
            src,
            comments,
            next_comment: 0,
        }
    }

    fn finish(mut self) -> String {
        // Flush any comments that trail the final item.
        self.flush_comments_before(usize::MAX, false);
        // Collapse trailing blank lines, ensure exactly one final newline.
        while self.out.ends_with('\n') {
            self.out.pop();
        }
        // Strip trailing whitespace on the last line too.
        while self.out.ends_with(' ') || self.out.ends_with('\t') {
            self.out.pop();
        }
        if !self.out.is_empty() {
            self.out.push('\n');
        }
        self.out
    }

    // ----- low level emit -----

    fn line(&mut self, text: &str) {
        if text.is_empty() {
            self.out.push('\n');
            return;
        }
        for _ in 0..self.indent {
            self.out.push_str(&self.indent_unit);
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn blank(&mut self) {
        // Collapse consecutive blanks: only emit if the last char is a
        // single newline (not already a blank line) and output is non-empty.
        if self.out.is_empty() {
            return;
        }
        if self.out.ends_with("\n\n") {
            return;
        }
        if self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn line_of(&self, byte: usize) -> usize {
        self.src[..byte.min(self.src.len())]
            .bytes()
            .filter(|b| *b == b'\n')
            .count()
    }

    // ----- comment weaving -----

    /// Emit every pending comment whose start byte is before `limit`.
    /// `as_leading` controls indentation: leading comments use the current
    /// indent and sit on their own lines.
    fn flush_comments_before(&mut self, limit: usize, leading: bool) {
        let _ = leading;
        while self.next_comment < self.comments.len() {
            let c = &self.comments[self.next_comment];
            if c.start >= limit {
                break;
            }
            let text = c.text.clone();
            self.line(&text);
            self.next_comment += 1;
        }
    }

    /// Take the trailing comment that begins on `line` (same source line as
    /// the item just emitted), if any, without emitting a newline first.
    fn take_trailing_comment(&mut self, line: usize) -> Option<String> {
        if self.next_comment < self.comments.len() {
            let c = &self.comments[self.next_comment];
            if c.own_line {
                return None;
            }
            if self.line_of(c.start) == line {
                let text = c.text.clone();
                self.next_comment += 1;
                return Some(text);
            }
        }
        None
    }

    // ----- top level -----

    fn file(&mut self, file: &File) {
        // Source line of the last thing emitted (item end or comment line),
        // used to decide blank-line separation. The canonical rule preserves
        // a single blank between top level entities when the source had a
        // gap, and collapses larger gaps to one.
        let mut prev_end_line: Option<usize> = None;
        for item in &file.items {
            // Emit any own-line comments that precede this item, each with a
            // gap-preserving blank, then the item itself.
            prev_end_line = self.emit_toplevel_comments(item.span.start, prev_end_line);
            self.maybe_blank(prev_end_line, item.span.start);
            let trailing = self.decl(item);
            if let Some(t) = trailing {
                self.attach_trailing(&t);
            }
            prev_end_line = Some(self.line_of(item.span.end));
        }
        // Trailing own-line comments after the last item.
        self.emit_toplevel_comments(usize::MAX, prev_end_line);
    }

    /// Emit own-line comments before `byte` at the current indent, inserting
    /// a single blank line whenever the source had a gap before a comment.
    /// Returns the updated `prev_end_line`.
    fn emit_toplevel_comments(
        &mut self,
        byte: usize,
        mut prev_end_line: Option<usize>,
    ) -> Option<usize> {
        loop {
            let Some(c) = self.comments.get(self.next_comment) else {
                break;
            };
            if c.start >= byte || !c.own_line {
                break;
            }
            let c_start = c.start;
            let text = c.text.clone();
            self.maybe_blank(prev_end_line, c_start);
            self.line(&text);
            prev_end_line = Some(self.line_of(c_start));
            self.next_comment += 1;
        }
        prev_end_line
    }

    fn attach_trailing(&mut self, comment: &str) {
        // Replace the trailing newline with " // ...".
        if self.out.ends_with('\n') {
            self.out.pop();
        }
        self.out.push(' ');
        self.out.push_str(comment);
        self.out.push('\n');
    }
}

// ----- declarations -----

impl Printer<'_> {
    /// Emit a declaration. Returns a trailing comment to attach on the same
    /// line, if one was found.
    fn decl(&mut self, decl: &Decl) -> Option<String> {
        match &decl.kind {
            DeclKind::Macro(m) => self.macro_decl(m),
            DeclKind::Function(f) => self.function(f, ""),
            DeclKind::Struct(s) => self.struct_decl(s),
            DeclKind::Trait(t) => self.trait_decl(t),
            DeclKind::Impl(i) => self.impl_decl(i),
            DeclKind::Enum(e) => self.enum_decl(e),
            DeclKind::Extern(e) => self.extern_decl(e),
            DeclKind::Import(im) => self.import_decl(im),
            DeclKind::Const(c) => self.const_decl(c),
            DeclKind::Let(l) => self.let_decl(l),
        }
    }

    /// Emit a `macro name { (matcher) => { template } ... }` definition. One
    /// rule renders on a single line; several rules each get their own
    /// indented line. The matcher and template token runs are rendered with
    /// canonical spacing.
    fn macro_decl(&mut self, m: &MacroDef) -> Option<String> {
        let rules = split_macro_rules(&m.body);
        if rules.is_empty() {
            // The body did not match the expected rule shape; render its
            // tokens inline so the output still round-trips and re-parses.
            let body = self.render_macro_tokens(&m.body);
            self.line(&format!("macro {} {{ {} }}", m.name, body));
        } else if rules.len() == 1 {
            let mt = self.render_macro_tokens(&rules[0].0);
            let tp = self.render_macro_tokens(&rules[0].1);
            self.line(&format!("macro {} {{ ({}) => {{ {} }} }}", m.name, mt, tp));
        } else {
            self.line(&format!("macro {} {{", m.name));
            self.indent += 1;
            for (matcher, template) in &rules {
                let mt = self.render_macro_tokens(matcher);
                let tp = self.render_macro_tokens(template);
                self.line(&format!("({}) => {{ {} }}", mt, tp));
            }
            self.indent -= 1;
            self.line("}");
        }
        self.take_trailing_comment(self.line_of(m.span.end))
    }

    /// The exact source text of a token.
    fn tok_src(&self, t: &Token) -> &str {
        self.src.get(t.span.start..t.span.end).unwrap_or("")
    }

    /// Render a macro matcher, template, or call-argument token run with
    /// canonical spacing. Metavariables (`$x`, `$x:expr`) and the repetition
    /// sigil `$(` are kept tight; other tokens use expression-like spacing.
    /// Layout tokens (newlines) are dropped. The result re-lexes to the same
    /// tokens, so formatting is idempotent.
    fn render_macro_tokens(&self, raw: &[Token]) -> String {
        let toks: Vec<&Token> = raw
            .iter()
            .filter(|t| !matches!(t.kind, TokenKind::Newline | TokenKind::Eof))
            .collect();
        let mut out = String::new();
        let mut prev: Option<TokenKind> = None;
        let mut i = 0;
        while i < toks.len() {
            let t = toks[i];
            if matches!(t.kind, TokenKind::Dollar) {
                if let Some(p) = &prev {
                    if macro_space_before(p, &TokenKind::Dollar) {
                        out.push(' ');
                    }
                }
                out.push('$');
                i += 1;
                // `$(` repetition group: keep the bracket tight.
                if i < toks.len() && matches!(toks[i].kind, TokenKind::LParen) {
                    out.push('(');
                    prev = Some(TokenKind::LParen);
                    i += 1;
                    continue;
                }
                // `$name`, optionally with a `:fragment` annotation.
                if i < toks.len() {
                    if let TokenKind::Identifier(_) = toks[i].kind {
                        out.push_str(self.tok_src(toks[i]));
                        i += 1;
                        if i + 1 < toks.len()
                            && matches!(toks[i].kind, TokenKind::Colon)
                            && matches!(toks[i + 1].kind, TokenKind::Identifier(_))
                        {
                            out.push(':');
                            out.push_str(self.tok_src(toks[i + 1]));
                            i += 2;
                        }
                        // A metavariable behaves like an atom for spacing.
                        prev = Some(TokenKind::Identifier(String::new()));
                        continue;
                    }
                }
                prev = Some(TokenKind::Dollar);
                continue;
            }
            if let Some(p) = &prev {
                if macro_space_before(p, &t.kind) {
                    out.push(' ');
                }
            }
            out.push_str(self.tok_src(t));
            prev = Some(t.kind.clone());
            i += 1;
        }
        out
    }

    fn function(&mut self, f: &Function, prefix: &str) -> Option<String> {
        let mut head = String::new();
        head.push_str(prefix);
        head.push_str("fun ");
        head.push_str(&f.name);
        head.push_str(&render_generics(&f.generics));
        head.push('(');
        head.push_str(&render_params(&f.params));
        head.push(')');
        if let Some(ret) = &f.ret {
            head.push_str(" -> ");
            head.push_str(&render_type(ret));
        }

        match &f.body {
            FunctionBody::None => {
                self.line(&head);
                let line = self.line_of(f.span.end);
                self.take_trailing_comment(line)
            }
            FunctionBody::Expr(e) => {
                head.push_str(" = ");
                head.push_str(&self.render_expr(e));
                self.line(&head);
                let line = self.line_of(f.span.end);
                self.take_trailing_comment(line)
            }
            FunctionBody::Block(b) => {
                if b.stmts.is_empty() && b.trailing.is_none() {
                    head.push_str(" {}");
                    self.line(&head);
                    let line = self.line_of(f.span.end);
                    return self.take_trailing_comment(line);
                }
                head.push_str(" {");
                self.line(&head);
                self.block_body(b);
                self.line("}");
                None
            }
        }
    }

    fn struct_decl(&mut self, s: &Struct) -> Option<String> {
        if s.repr_c {
            self.line("@repr(C)");
        }
        if !s.derives.is_empty() {
            self.line(&format!("@derive({})", s.derives.join(", ")));
        }
        let mut head = String::from("struct ");
        head.push_str(&s.name);
        head.push_str(&render_generics(&s.generics));
        if s.fields.is_empty() {
            head.push_str(" {}");
            self.line(&head);
            return None;
        }
        head.push_str(" {");
        self.line(&head);
        self.indent += 1;
        for field in &s.fields {
            self.struct_field_line(field);
        }
        self.indent -= 1;
        self.line("}");
        None
    }

    fn struct_field_line(&mut self, field: &StructField) {
        let line = self.line_of(field.span.start);
        self.emit_indented_comments_before(field.span.start, line);
        let text = format!("{}: {},", field.name, render_type(&field.ty));
        self.line(&text);
        if let Some(t) = self.take_trailing_comment(self.line_of(field.span.end)) {
            self.attach_trailing(&t);
        }
    }

    fn trait_decl(&mut self, t: &Trait) -> Option<String> {
        let mut head = String::from("trait ");
        head.push_str(&t.name);
        head.push_str(&render_generics(&t.generics));
        if t.members.is_empty() {
            head.push_str(" {}");
            self.line(&head);
            return None;
        }
        head.push_str(" {");
        self.line(&head);
        self.indent += 1;
        for (i, m) in t.members.iter().enumerate() {
            if i > 0 {
                self.blank();
            }
            self.emit_indented_comments_before(m.span.start, self.line_of(m.span.start));
            if let Some(tr) = self.function(m, "") {
                self.attach_trailing(&tr);
            }
        }
        self.indent -= 1;
        self.line("}");
        None
    }

    fn impl_decl(&mut self, im: &Impl) -> Option<String> {
        let mut head = String::from("impl");
        head.push_str(&render_generics(&im.generics));
        head.push(' ');
        head.push_str(&render_type_path(&im.trait_or_type));
        if let Some(for_ty) = &im.for_type {
            head.push_str(" for ");
            head.push_str(&render_type_path(for_ty));
        }
        if im.items.is_empty() {
            head.push_str(" {}");
            self.line(&head);
            return None;
        }
        head.push_str(" {");
        self.line(&head);
        self.indent += 1;
        for (i, m) in im.items.iter().enumerate() {
            if i > 0 {
                self.blank();
            }
            self.emit_indented_comments_before(m.span.start, self.line_of(m.span.start));
            if let Some(tr) = self.function(m, "") {
                self.attach_trailing(&tr);
            }
        }
        self.indent -= 1;
        self.line("}");
        None
    }

    fn enum_decl(&mut self, e: &Enum) -> Option<String> {
        if !e.derives.is_empty() {
            self.line(&format!("@derive({})", e.derives.join(", ")));
        }
        let mut head = String::from("enum ");
        head.push_str(&e.name);
        head.push_str(&render_generics(&e.generics));
        if e.variants.is_empty() {
            head.push_str(" {}");
            self.line(&head);
            return None;
        }
        head.push_str(" {");
        self.line(&head);
        self.indent += 1;
        for v in &e.variants {
            self.emit_indented_comments_before(v.span.start, self.line_of(v.span.start));
            let mut text = v.name.clone();
            match &v.payload {
                VariantPayload::Unit => {}
                VariantPayload::Tuple(tys) => {
                    text.push('(');
                    let parts: Vec<String> = tys.iter().map(render_type).collect();
                    text.push_str(&parts.join(", "));
                    text.push(')');
                }
                VariantPayload::Struct(fields) => {
                    text.push('(');
                    let parts: Vec<String> = fields
                        .iter()
                        .map(|f| format!("{}: {}", f.name, render_type(&f.ty)))
                        .collect();
                    text.push_str(&parts.join(", "));
                    text.push(')');
                }
            }
            text.push(',');
            self.line(&text);
            if let Some(t) = self.take_trailing_comment(self.line_of(v.span.end)) {
                self.attach_trailing(&t);
            }
        }
        self.indent -= 1;
        self.line("}");
        None
    }

    fn extern_decl(&mut self, e: &Extern) -> Option<String> {
        let head = format!("extern \"{}\" {{", e.abi);
        self.line(&head);
        self.indent += 1;
        for it in &e.items {
            self.emit_indented_comments_before(it.span.start, self.line_of(it.span.start));
            self.extern_fn_line(it);
        }
        self.indent -= 1;
        self.line("}");
        None
    }

    fn extern_fn_line(&mut self, it: &ExternFn) {
        let mut text = String::from("fun ");
        text.push_str(&it.name);
        text.push('(');
        text.push_str(&render_params(&it.params));
        text.push(')');
        if let Some(ret) = &it.ret {
            text.push_str(" -> ");
            text.push_str(&render_type(ret));
        }
        self.line(&text);
        if let Some(t) = self.take_trailing_comment(self.line_of(it.span.end)) {
            self.attach_trailing(&t);
        }
    }

    fn import_decl(&mut self, im: &Import) -> Option<String> {
        let mut text = String::from("import ");
        match &im.source {
            ImportSource::Std(parts) => {
                text.push_str("std");
                for part in parts {
                    text.push('/');
                    text.push_str(part);
                }
            }
            ImportSource::Quoted(s) => {
                text.push('"');
                text.push_str(s);
                text.push('"');
            }
        }
        if let Some(alias) = &im.alias {
            text.push_str(" as ");
            text.push_str(alias);
        }
        if !im.selectors.is_empty() {
            text.push_str(" { ");
            text.push_str(&im.selectors.join(", "));
            text.push_str(" }");
        }
        self.line(&text);
        self.take_trailing_comment(self.line_of(im.span.end))
    }

    fn const_decl(&mut self, c: &Const) -> Option<String> {
        let text = format!(
            "const {}: {} = {}",
            c.name,
            render_type(&c.ty),
            self.render_expr(&c.value)
        );
        self.line(&text);
        self.take_trailing_comment(self.line_of(c.span.end))
    }

    fn let_decl(&mut self, l: &LetDecl) -> Option<String> {
        // A module-level `let` declaration is always mutable in spelling.
        let text = self.render_let(&l.name, &l.ty, &l.init, true);
        self.line(&text);
        self.take_trailing_comment(self.line_of(l.span.end))
    }

    fn render_let(
        &mut self,
        name: &str,
        ty: &Option<Type>,
        init: &Option<Expr>,
        mutable: bool,
    ) -> String {
        let mut text = String::from(if mutable { "let " } else { "const " });
        text.push_str(name);
        if let Some(t) = ty {
            text.push_str(": ");
            text.push_str(&render_type(t));
        }
        if let Some(e) = init {
            let e = self.render_expr(e);
            text.push_str(" = ");
            text.push_str(&e);
        }
        text
    }

    /// Emit own-line comments before `byte`, at the current indent.
    fn emit_indented_comments_before(&mut self, byte: usize, _item_line: usize) {
        while self.next_comment < self.comments.len() {
            let c = &self.comments[self.next_comment];
            if c.start >= byte || !c.own_line {
                break;
            }
            let text = c.text.clone();
            self.line(&text);
            self.next_comment += 1;
        }
    }
}

// ----- statements and blocks -----

impl Printer<'_> {
    /// Emit the statements of a block at one deeper indent level. Used for
    /// function, loop, if, while, for bodies.
    fn block_body(&mut self, b: &Block) {
        self.indent += 1;
        self.stmts(b);
        self.indent -= 1;
    }

    fn stmts(&mut self, b: &Block) {
        let mut prev_end_line: Option<usize> = None;
        for (i, s) in b.stmts.iter().enumerate() {
            if i > 0 {
                self.maybe_blank(prev_end_line, s.span.start);
            }
            self.emit_stmt_leading_comments(s.span.start);
            self.stmt(s);
            prev_end_line = Some(self.line_of(s.span.end));
        }
        if let Some(t) = &b.trailing {
            if !b.stmts.is_empty() {
                self.maybe_blank(prev_end_line, t.span.start);
            }
            self.emit_stmt_leading_comments(t.span.start);
            let text = self.render_expr(t);
            self.emit_multiline(&text);
            if let Some(c) = self.take_trailing_comment(self.line_of(t.span.end)) {
                self.attach_trailing(&c);
            }
        }
    }

    /// Emit one blank line when the source separated `prev_end_line` from the
    /// next item by at least one blank line, counting the first own-line
    /// leading comment (if any) as the start of the next item so a comment
    /// glued to its item does not drift across format passes.
    fn maybe_blank(&mut self, prev_end_line: Option<usize>, next_byte: usize) {
        let Some(pe) = prev_end_line else {
            return;
        };
        let anchor = self.leading_anchor_line(next_byte);
        if anchor > pe + 1 {
            self.blank();
        }
    }

    /// Source line of the first own-line comment preceding `byte` (without
    /// consuming it), or the line of `byte` when there is none.
    fn leading_anchor_line(&self, byte: usize) -> usize {
        if let Some(c) = self.comments.get(self.next_comment) {
            if c.start < byte && c.own_line {
                return self.line_of(c.start);
            }
        }
        self.line_of(byte)
    }

    fn emit_stmt_leading_comments(&mut self, byte: usize) {
        loop {
            let Some(c) = self.comments.get(self.next_comment) else {
                break;
            };
            if c.start >= byte || !c.own_line {
                break;
            }
            let text = c.text.clone();
            self.line(&text);
            self.next_comment += 1;
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        let end_line = self.line_of(s.span.end);
        match &s.kind {
            StmtKind::Let {
                name,
                ty,
                init,
                mutable,
            } => {
                let text = self.render_let(name, ty, init, *mutable);
                self.emit_multiline(&text);
            }
            StmtKind::Return(e) => {
                let text = match e {
                    Some(e) => format!("return {}", self.render_expr(e)),
                    None => "return".to_string(),
                };
                self.emit_multiline(&text);
            }
            StmtKind::Break(e) => {
                let text = match e {
                    Some(e) => format!("break {}", self.render_expr(e)),
                    None => "break".to_string(),
                };
                self.emit_multiline(&text);
            }
            StmtKind::Continue => self.line("continue"),
            StmtKind::Defer(e) => {
                let text = format!("defer {}", self.render_expr(e));
                self.emit_multiline(&text);
            }
            StmtKind::Spawn(e) => {
                // `spawn` reads as a call: no space before the argument and a
                // single layer of parentheses. Unwrap an explicit paren so the
                // canonical form `spawn(<closure>)` is stable under repeated
                // formatting.
                let inner = match &e.kind {
                    ExprKind::Paren(inner) => inner.as_ref(),
                    _ => e,
                };
                let text = format!("spawn({})", self.render_expr(inner));
                self.emit_multiline(&text);
            }
            StmtKind::Assign { target, op, value } => {
                let text = format!(
                    "{} {} {}",
                    self.render_expr(target),
                    assign_op(*op),
                    self.render_expr(value)
                );
                self.emit_multiline(&text);
            }
            StmtKind::Expr(e) => {
                let text = self.render_expr(e);
                self.emit_multiline(&text);
            }
        }
        if let Some(c) = self.take_trailing_comment(end_line) {
            self.attach_trailing(&c);
        }
    }

    /// Emit text that may already contain newlines (block-bearing
    /// expressions render with embedded newlines and indentation baked in
    /// by `render_expr`). The first line gets the current indent; following
    /// lines are emitted verbatim because they were produced with absolute
    /// indentation already applied.
    fn emit_multiline(&mut self, text: &str) {
        if !text.contains('\n') {
            self.line(text);
            return;
        }
        // Multi-line renders are produced with their own indentation relative
        // to indent 0; shift every line by the current indent.
        for (i, part) in text.split('\n').enumerate() {
            if i > 0 {
                self.out.push('\n');
            }
            if part.is_empty() {
                continue;
            }
            for _ in 0..self.indent {
                self.out.push_str(&self.indent_unit);
            }
            self.out.push_str(part);
        }
        self.out.push('\n');
    }
}

// ----- expressions -----
//
// Expressions render to a `String`. Block-bearing expressions (if, match,
// while, loop, for, block, lambda-with-block) render multi-line, indented
// relative to column zero; `emit_multiline` shifts them by the enclosing
// indent. Leaf and operator expressions render on one line.

impl Printer<'_> {
    fn render_expr(&mut self, e: &Expr) -> String {
        self.expr_at(e, 0)
    }

    /// Render an expression whose block continuations should be indented at
    /// `base` levels relative to column zero. Comments inside nested blocks
    /// are woven in from the shared cursor as the block renders.
    fn expr_at(&mut self, e: &Expr, base: usize) -> String {
        match &e.kind {
            ExprKind::MacroCall(m) => {
                let (open, close) = macro_delim_chars(m.delim);
                format!(
                    "{}!{}{}{}",
                    m.name,
                    open,
                    self.render_macro_tokens(&m.tokens),
                    close
                )
            }
            ExprKind::Int(n) => n.to_string(),
            ExprKind::Float(v) => render_float(*v),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(s) => render_string_lit(s),
            ExprKind::InterpolatedString(frags) => self.render_interpolated(frags),
            ExprKind::BlockStr(s) => format!("\"\"\"{}\"\"\"", s),
            ExprKind::Char(c) => format!("'{}'", render_char(*c)),
            ExprKind::CStr(s) => format!("c{}", render_string_lit(s)),
            ExprKind::SelfLower => "self".to_string(),
            ExprKind::SelfUpper => "Self".to_string(),
            ExprKind::Ident { name, generics } => {
                let mut s = name.clone();
                if !generics.is_empty() {
                    s.push_str(&render_type_args(generics));
                }
                s
            }
            ExprKind::StructLit {
                name,
                generics,
                fields,
            } => {
                let mut s = name.clone();
                if !generics.is_empty() {
                    s.push_str(&render_type_args(generics));
                }
                if fields.is_empty() {
                    s.push_str(" {}");
                    return s;
                }
                let multiline = self.spans_multiple_lines(e.span.start, e.span.end);
                if multiline {
                    s.push_str(" {\n");
                    for f in fields {
                        for _ in 0..base + 1 {
                            s.push_str(&self.indent_unit);
                        }
                        let value = self.field_value(f);
                        if is_field_shorthand(&f.name, &f.value) {
                            let _ = writeln!(s, "{},", f.name);
                        } else {
                            let _ = writeln!(s, "{}: {},", f.name, value);
                        }
                    }
                    for _ in 0..base {
                        s.push_str(&self.indent_unit);
                    }
                    s.push('}');
                } else {
                    s.push_str(" { ");
                    let mut parts: Vec<String> = Vec::with_capacity(fields.len());
                    for f in fields {
                        if is_field_shorthand(&f.name, &f.value) {
                            parts.push(f.name.clone());
                        } else {
                            let v = self.field_value(f);
                            parts.push(format!("{}: {}", f.name, v));
                        }
                    }
                    s.push_str(&parts.join(", "));
                    s.push_str(" }");
                }
                s
            }
            ExprKind::Array(items) => {
                let parts = self.expr_list(items, base);
                format!("[{}]", parts.join(", "))
            }
            ExprKind::Tuple(items) => {
                let parts = self.expr_list(items, base);
                format!("({})", parts.join(", "))
            }
            ExprKind::SetLit(items) => {
                let parts = self.expr_list(items, base);
                // A single-element set keeps a trailing comma so it does
                // not re-parse as a one-expression block `{ x }`.
                if parts.len() == 1 {
                    format!("{{{},}}", parts[0])
                } else {
                    format!("{{{}}}", parts.join(", "))
                }
            }
            ExprKind::MapLit(pairs) => {
                if pairs.is_empty() {
                    return "[:]".to_string();
                }
                let parts: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("{}: {}", self.expr_at(k, base), self.expr_at(v, base)))
                    .collect();
                format!("[{}]", parts.join(", "))
            }
            ExprKind::Paren(inner) => {
                let inner = self.expr_at(inner, base);
                format!("({})", inner)
            }
            ExprKind::Block(b) => self.render_block(b, base),
            ExprKind::Unary { op, operand } => {
                let operand = self.expr_at(operand, base);
                format!("{}{}", unary_op(*op), operand)
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let l = self.expr_at(lhs, base);
                let r = self.expr_at(rhs, base);
                format!("{} {} {}", l, binary_op(*op), r)
            }
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                let sep = if *inclusive { "..=" } else { ".." };
                let s = self.expr_at(start, base);
                let e = self.expr_at(end, base);
                format!("{}{}{}", s, sep, e)
            }
            ExprKind::Call { callee, args } => {
                let callee = self.expr_at(callee, base);
                let parts = self.expr_list(args, base);
                format!("{}({})", callee, parts.join(", "))
            }
            ExprKind::MethodCall {
                receiver,
                name,
                generics,
                args,
            } => {
                let mut s = self.expr_at(receiver, base);
                s.push('.');
                s.push_str(name);
                if !generics.is_empty() {
                    s.push_str(&render_type_args(generics));
                }
                let parts = self.expr_list(args, base);
                let _ = write!(s, "({})", parts.join(", "));
                s
            }
            ExprKind::Field { receiver, name } => {
                let r = self.expr_at(receiver, base);
                format!("{}.{}", r, name)
            }
            ExprKind::Index { receiver, index } => {
                let r = self.expr_at(receiver, base);
                let i = self.expr_at(index, base);
                format!("{}[{}]", r, i)
            }
            ExprKind::Try(inner) => {
                let inner = self.expr_at(inner, base);
                format!("{}?", inner)
            }
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.render_if(cond, then_branch, else_branch, base),
            ExprKind::Match { scrutinee, arms } => self.render_match(scrutinee, arms, base),
            ExprKind::Loop(b) => {
                let body = self.render_block(b, base);
                format!("loop {}", body)
            }
            ExprKind::While { cond, body } => {
                let cond = self.expr_at(cond, base);
                let body = self.render_block(body, base);
                format!("while {} {}", cond, body)
            }
            ExprKind::For {
                pattern,
                iter,
                body,
            } => {
                let pat = render_pattern(pattern);
                let iter = self.expr_at(iter, base);
                let body = self.render_block(body, base);
                format!("for {} in {} {}", pat, iter, body)
            }
            ExprKind::Lambda {
                params,
                ret,
                body,
                params_inferred,
            } => self.render_lambda(params, ret, body, *params_inferred, base),
        }
    }

    fn expr_list(&mut self, items: &[Expr], base: usize) -> Vec<String> {
        let mut parts = Vec::with_capacity(items.len());
        for it in items {
            parts.push(self.expr_at(it, base));
        }
        parts
    }

    fn field_value(&mut self, f: &crate::ast::FieldInit) -> String {
        self.render_expr(&f.value)
    }

    fn spans_multiple_lines(&self, start: usize, end: usize) -> bool {
        self.line_of(start) != self.line_of(end)
    }

    fn render_interpolated(&mut self, frags: &[StrFragment]) -> String {
        let mut s = String::from("\"");
        for frag in frags {
            match frag {
                StrFragment::Literal(text) => s.push_str(&escape_str_body(text)),
                StrFragment::Expr(e) => {
                    let inner = self.render_expr(e);
                    s.push_str("${");
                    s.push_str(&inner);
                    s.push('}');
                }
            }
        }
        s.push('"');
        s
    }

    /// Render the statements of a block into a captured string at `indent`,
    /// drawing comments from the shared cursor by source position.
    fn capture_stmts(&mut self, b: &Block, indent: usize) -> String {
        let saved_out = std::mem::take(&mut self.out);
        let saved_indent = self.indent;
        self.indent = indent;
        self.stmts(b);
        self.indent = saved_indent;
        let inner = std::mem::replace(&mut self.out, saved_out);
        inner.trim_end_matches('\n').to_string()
    }

    fn render_block(&mut self, b: &Block, base: usize) -> String {
        if b.stmts.is_empty() && b.trailing.is_none() {
            return "{}".to_string();
        }
        let inner = self.capture_stmts(b, base + 1);
        let mut s = String::from("{\n");
        s.push_str(&inner);
        s.push('\n');
        for _ in 0..base {
            s.push_str(&self.indent_unit);
        }
        s.push('}');
        s
    }

    fn render_if(
        &mut self,
        cond: &Expr,
        then_branch: &Block,
        else_branch: &Option<Box<ElseBranch>>,
        base: usize,
    ) -> String {
        let cond = self.expr_at(cond, base);
        let then_block = self.render_block(then_branch, base);
        let mut s = format!("if {} {}", cond, then_block);
        if let Some(eb) = else_branch {
            match eb.as_ref() {
                ElseBranch::If(e) => {
                    let e = self.expr_at(e, base);
                    s.push_str(" else ");
                    s.push_str(&e);
                }
                ElseBranch::Block(b) => {
                    let b = self.render_block(b, base);
                    s.push_str(" else ");
                    s.push_str(&b);
                }
            }
        }
        s
    }

    fn render_match(&mut self, scrutinee: &Expr, arms: &[MatchArm], base: usize) -> String {
        let scrut = self.expr_at(scrutinee, base);
        let mut s = format!("match {} {{\n", scrut);
        for arm in arms {
            let pat = render_pattern(&arm.pattern);
            let guard = arm.guard.as_ref().map(|g| self.expr_at(g, base + 1));
            let body = self.expr_at(&arm.body, base + 1);
            for _ in 0..base + 1 {
                s.push_str(&self.indent_unit);
            }
            s.push_str(&pat);
            if let Some(g) = guard {
                s.push_str(" if ");
                s.push_str(&g);
            }
            s.push_str(" -> ");
            s.push_str(&body);
            s.push_str(",\n");
        }
        for _ in 0..base {
            s.push_str(&self.indent_unit);
        }
        s.push('}');
        s
    }

    fn render_lambda(
        &mut self,
        params: &[LambdaParam],
        ret: &Option<Type>,
        body: &LambdaBody,
        params_inferred: bool,
        base: usize,
    ) -> String {
        if params_inferred {
            // Shorthand `{ x, y -> body }`. The parser always stores the body
            // as a block. When that block is a single trailing expression
            // with no statements, render it inline; otherwise render the
            // statements between the arrow and the closing brace.
            let names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
            let head = if names.is_empty() {
                "{ ->".to_string()
            } else {
                format!("{{ {} ->", names.join(", "))
            };
            let LambdaBody::Block(b) = body else {
                // Defensive: shorthand always carries a block.
                let body_str = match body {
                    LambdaBody::Expr(e) => self.expr_at(e, base),
                    LambdaBody::Block(b) => self.render_block(b, base),
                };
                return format!("{} {} }}", head, body_str);
            };
            if b.stmts.is_empty() {
                match &b.trailing {
                    Some(t) => {
                        let t = self.expr_at(t, base);
                        format!("{} {} }}", head, t)
                    }
                    None => format!("{} }}", head),
                }
            } else {
                let inner = self.capture_stmts(b, base + 1);
                let mut s = format!("{}\n", head);
                s.push_str(&inner);
                s.push('\n');
                for _ in 0..base {
                    s.push_str(&self.indent_unit);
                }
                s.push('}');
                s
            }
        } else {
            let mut s = String::from("fun(");
            let parts: Vec<String> = params
                .iter()
                .map(|p| match &p.ty {
                    Some(t) => format!("{}: {}", p.name, render_type(t)),
                    None => p.name.clone(),
                })
                .collect();
            s.push_str(&parts.join(", "));
            s.push(')');
            if let Some(r) = ret {
                s.push_str(" -> ");
                s.push_str(&render_type(r));
            }
            match body {
                LambdaBody::Expr(e) => {
                    let e = self.expr_at(e, base);
                    s.push_str(" = ");
                    s.push_str(&e);
                }
                LambdaBody::Block(b) => {
                    let b = self.render_block(b, base);
                    s.push(' ');
                    s.push_str(&b);
                }
            }
            s
        }
    }
}

fn is_field_shorthand(name: &str, value: &Expr) -> bool {
    matches!(&value.kind, ExprKind::Ident { name: n, generics } if n == name && generics.is_empty())
}

// ----- shared free-function renderers (no comment state) -----

fn render_generics(gens: &[GenericParam]) -> String {
    if gens.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = gens
        .iter()
        .map(|g| {
            if g.bounds.is_empty() {
                g.name.clone()
            } else {
                let bounds: Vec<String> = g.bounds.iter().map(render_type_path).collect();
                format!("{}: {}", g.name, bounds.join(" + "))
            }
        })
        .collect();
    format!("<{}>", parts.join(", "))
}

fn render_params(params: &[Param]) -> String {
    params
        .iter()
        .map(render_param)
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_param(p: &Param) -> String {
    // `self` parameter: rendered bare when its type is the implicit Self.
    if p.name == "self" {
        if let TypeKind::Path(path) = &p.ty.kind {
            if path.segments.len() == 1 && path.segments[0].name == "Self" {
                return "self".to_string();
            }
        }
    }
    format!("{}: {}", p.name, render_type(&p.ty))
}

/// The opening and closing bracket characters for a macro call delimiter.
fn macro_delim_chars(d: MacroDelim) -> (char, char) {
    match d {
        MacroDelim::Paren => ('(', ')'),
        MacroDelim::Bracket => ('[', ']'),
        MacroDelim::Brace => ('{', '}'),
    }
}

/// Split a macro body (the tokens between the outer braces) into its rules,
/// each a `(matcher, template)` pair of inner token runs. Returns an empty
/// vector when the body does not match the `(...) => { ... } ...` shape, so
/// the caller can fall back to rendering the body inline.
fn split_macro_rules(body: &[Token]) -> Vec<(Vec<Token>, Vec<Token>)> {
    let toks: Vec<&Token> = body
        .iter()
        .filter(|t| !matches!(t.kind, TokenKind::Newline | TokenKind::Eof))
        .collect();
    let mut rules = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let Some((matcher, after_m)) =
            capture_macro_group(&toks, i, &TokenKind::LParen, &TokenKind::RParen)
        else {
            return Vec::new();
        };
        i = after_m;
        if i >= toks.len() || !matches!(toks[i].kind, TokenKind::FatArrow) {
            return Vec::new();
        }
        i += 1;
        let Some((template, after_t)) =
            capture_macro_group(&toks, i, &TokenKind::LBrace, &TokenKind::RBrace)
        else {
            return Vec::new();
        };
        i = after_t;
        rules.push((matcher, template));
    }
    rules
}

/// Capture the tokens inside a bracket group that begins at `start` (which
/// must be `open`), tracking nesting of the same bracket pair. Returns the
/// inner tokens (cloned, brackets excluded) and the index just past the
/// closing bracket, or `None` when `start` is not `open` or the group never
/// closes.
fn capture_macro_group(
    toks: &[&Token],
    start: usize,
    open: &TokenKind,
    close: &TokenKind,
) -> Option<(Vec<Token>, usize)> {
    if start >= toks.len() || &toks[start].kind != open {
        return None;
    }
    let mut depth = 0usize;
    let mut inner = Vec::new();
    let mut i = start;
    while i < toks.len() {
        let k = &toks[i].kind;
        if k == open {
            depth += 1;
            if depth > 1 {
                inner.push(toks[i].clone());
            }
        } else if k == close {
            depth -= 1;
            if depth == 0 {
                return Some((inner, i + 1));
            }
            inner.push(toks[i].clone());
        } else {
            inner.push(toks[i].clone());
        }
        i += 1;
    }
    None
}

/// Whether a space goes between two macro tokens. Default is a space, with
/// the usual tight cases: no space inside call/index brackets, before
/// punctuation, or after `.`/`$`/`!`.
fn macro_space_before(prev: &TokenKind, cur: &TokenKind) -> bool {
    use TokenKind::*;
    if matches!(
        cur,
        Comma | Semi | RParen | RBracket | RBrace | Dot | ColonColon | Question | Colon | Bang
    ) {
        return false;
    }
    if matches!(prev, LParen | LBracket | Dot | ColonColon | Dollar | Bang) {
        return false;
    }
    // `name(` / `name[` / `)(` stay tight (call or index).
    if matches!(cur, LParen | LBracket)
        && matches!(
            prev,
            Identifier(_) | RParen | RBracket | SelfLower | SelfUpper
        )
    {
        return false;
    }
    true
}

fn render_type(ty: &Type) -> String {
    match &ty.kind {
        TypeKind::Path(p) => render_type_path(p),
        TypeKind::Optional(inner) => format!("{}?", render_type(inner)),
        TypeKind::Dyn(p) => format!("dyn {}", render_type_path(p)),
        TypeKind::Unit => "()".to_string(),
        TypeKind::Function { params, ret } => {
            let parts: Vec<String> = params.iter().map(render_type).collect();
            format!("fun({}) -> {}", parts.join(", "), render_type(ret))
        }
    }
}

fn render_type_path(p: &TypePath) -> String {
    let mut out = String::new();
    for (i, seg) in p.segments.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        out.push_str(&seg.name);
        if !seg.generics.is_empty() {
            out.push_str(&render_type_args(&seg.generics));
        }
    }
    out
}

fn render_type_args(args: &[Type]) -> String {
    let parts: Vec<String> = args.iter().map(render_type).collect();
    format!("<{}>", parts.join(", "))
}

fn render_pattern(pat: &Pattern) -> String {
    match &pat.kind {
        PatternKind::Wildcard => "_".to_string(),
        PatternKind::Literal(LiteralPattern::Int(n)) => n.to_string(),
        PatternKind::Literal(LiteralPattern::Float(v)) => render_float(*v),
        PatternKind::Literal(LiteralPattern::Bool(b)) => b.to_string(),
        PatternKind::Literal(LiteralPattern::String(s)) => render_string_lit(s),
        PatternKind::Literal(LiteralPattern::Char(c)) => format!("'{}'", render_char(*c)),
        PatternKind::Ident(name) => name.clone(),
        PatternKind::Tuple { name, elements } => {
            let parts: Vec<String> = elements.iter().map(render_pattern).collect();
            match name {
                Some(n) => format!("{}({})", n, parts.join(", ")),
                None => format!("({})", parts.join(", ")),
            }
        }
        PatternKind::Struct { name, fields } => {
            let parts: Vec<String> = fields
                .iter()
                .map(|f| match &f.pattern {
                    Some(p) => format!("{}: {}", f.name, render_pattern(p)),
                    None => f.name.clone(),
                })
                .collect();
            if parts.is_empty() {
                format!("{} {{}}", name)
            } else {
                format!("{} {{ {} }}", name, parts.join(", "))
            }
        }
        PatternKind::Range { lo, hi, inclusive } => {
            let sep = if *inclusive { "..=" } else { ".." };
            format!("{}{}{}", lo, sep, hi)
        }
    }
}

fn render_float(v: f64) -> String {
    if v.is_infinite() || v.is_nan() {
        // Not representable as a literal; emit a best-effort token. The
        // parser never produces these from literals, so this is unreachable
        // in practice.
        return format!("{}", v);
    }
    let s = format!("{}", v);
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{}.0", s)
    }
}

fn render_char(c: char) -> String {
    match c {
        '\n' => "\\n".to_string(),
        '\t' => "\\t".to_string(),
        '\r' => "\\r".to_string(),
        '\\' => "\\\\".to_string(),
        '\'' => "\\'".to_string(),
        '\0' => "\\0".to_string(),
        c if (c as u32) < 0x20 => format!("\\u{{{:x}}}", c as u32),
        c => c.to_string(),
    }
}

/// Render a plain string literal value back to a quoted, escaped literal.
fn render_string_lit(s: &str) -> String {
    format!("\"{}\"", escape_str_body(s))
}

/// Escape a string body for emission inside double quotes. A bare `$`
/// directly before `{` is escaped to `\$` so the result does not re-parse
/// as an interpolation.
fn escape_str_body(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    let chars: Vec<char> = s.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            '$' if chars.get(i + 1) == Some(&'{') => out.push_str("\\$"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn unary_op(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Neg => "-",
        UnaryOp::Not => "!",
        UnaryOp::Ref => "&",
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

#[cfg(test)]
mod tests;
