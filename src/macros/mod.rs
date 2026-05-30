//! Declarative macro expansion over the token stream.
//!
//! This pass sits between the lexer and the parser. It scans the token
//! stream for `macro` definitions, records their rules (a matcher token
//! pattern and a template token sequence), strips the definitions, then
//! rewrites every `name!(...)` invocation by matching the argument tokens
//! against a rule and splicing the captured token runs into a copy of the
//! template. The rewritten stream is then parsed normally.
//!
//! Scope of this slice (see `docs/v2/specs/macros.md`):
//!
//! * Definition: `macro <name> { (<matcher>) => { <template> } ... }`, one
//!   or more rules, first matching rule wins.
//! * Invocation: `<name>!(<tokens>)` in expression position.
//! * Metavariables: `$x:expr` (captures a balanced token run up to the next
//!   matcher delimiter) and `$x:ident` (captures one identifier token).
//! * No repetition, no hygiene, no other fragment kinds. Expansion is plain
//!   token substitution, so a captured name can shadow or be shadowed by a
//!   name in the template. Hygiene is a follow-up.
//!
//! The pass is a strict no-op when the source defines no macros: in that
//! case the input tokens are returned unchanged, so non-macro programs are
//! completely unaffected.

use std::collections::HashMap;

use crate::error::{ParseError, RavenError};
use crate::lexer::{Token, TokenKind};
use crate::span::Span;

/// Upper bound on expansion passes before the expander reports a likely
/// infinite macro. Each pass rewrites every outermost call once, so the
/// bound also limits recursion depth of macros that expand to other calls.
const EXPANSION_LIMIT: usize = 128;

/// A single metavariable fragment kind supported by this slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fragment {
    Expr,
    Ident,
}

/// One element of a matcher: either a bound metavariable or a literal token
/// that must appear verbatim in the argument stream.
#[derive(Debug, Clone)]
enum MatchItem {
    Meta { name: String, frag: Fragment },
    Literal(TokenKind),
}

/// One template element: either a metavariable splice or a verbatim token.
#[derive(Debug, Clone)]
enum TemplateItem {
    Meta(String),
    Token(Token),
}

/// One `(matcher) => { template }` arm.
#[derive(Debug, Clone)]
struct Rule {
    matcher: Vec<MatchItem>,
    template: Vec<TemplateItem>,
}

/// A named macro with its ordered rules.
#[derive(Debug, Clone)]
struct MacroDef {
    rules: Vec<Rule>,
    /// Span of the `macro` keyword, for duplicate-definition errors.
    span: Span,
}

/// Expand all declarative macros in `tokens`.
///
/// Returns the rewritten token stream (still ending in `Eof`). When the
/// source defines no macros the input is returned unchanged.
pub fn expand_tokens(tokens: &[Token]) -> Result<Vec<Token>, RavenError> {
    if !has_macro_keyword(tokens) {
        return Ok(tokens.to_vec());
    }

    let (defs, body) = collect_defs(tokens)?;
    let mut stream = body;
    // Tokens produced by expansion get fresh, unique byte ranges so that the
    // resolver, which keys identifier use sites by (file, start, end), never
    // sees two distinct expanded uses collide on the same span. Start the
    // synthetic range allocator above every real source offset.
    let mut spans = SpanGen::starting_after(tokens);
    let mut passes = 0;
    while contains_call(&stream) {
        passes += 1;
        if passes > EXPANSION_LIMIT {
            let span = stream
                .first()
                .map(|t| t.span.clone())
                .unwrap_or_else(|| tokens[0].span.clone());
            return Err(err(
                span,
                format!(
                    "macro expansion exceeded {} passes; a macro is likely recursive",
                    EXPANSION_LIMIT
                ),
            ));
        }
        stream = expand_once(&stream, &defs, &mut spans)?;
    }
    Ok(stream)
}

/// Allocator of unique synthetic byte ranges for expanded tokens. The
/// `line`/`col` of the originating call site are preserved for diagnostics;
/// only the byte range is made unique so use-site keys stay distinct.
struct SpanGen {
    next: usize,
}

impl SpanGen {
    fn starting_after(tokens: &[Token]) -> Self {
        let max_end = tokens.iter().map(|t| t.span.end).max().unwrap_or(0);
        SpanGen {
            next: max_end.saturating_add(1),
        }
    }

    /// A fresh one-byte span that borrows the file, line, and column of
    /// `at` but occupies a byte range used by nothing else.
    fn fresh(&mut self, at: &Span) -> Span {
        let start = self.next;
        self.next = self.next.saturating_add(1);
        Span::new(at.file.clone(), start, start + 1, at.line, at.col)
    }
}

/// True when an item-position `macro` keyword appears. `macro` is a
/// contextual identifier, so we only treat it as the keyword when it begins
/// a definition shape (`macro <ident> {`).
fn has_macro_keyword(tokens: &[Token]) -> bool {
    tokens.windows(3).any(|w| {
        is_macro_kw(&w[0].kind)
            && matches!(w[1].kind, TokenKind::Identifier(_))
            && matches!(w[2].kind, TokenKind::LBrace)
    })
}

fn is_macro_kw(kind: &TokenKind) -> bool {
    matches!(kind, TokenKind::Identifier(s) if s == "macro")
}

/// Collect every macro definition and return the remaining token stream with
/// the definitions removed.
fn collect_defs(tokens: &[Token]) -> Result<(HashMap<String, MacroDef>, Vec<Token>), RavenError> {
    let mut defs: HashMap<String, MacroDef> = HashMap::new();
    let mut body: Vec<Token> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        let is_def = is_macro_kw(&tok.kind)
            && matches!(
                tokens.get(i + 1).map(|t| &t.kind),
                Some(TokenKind::Identifier(_))
            )
            && matches!(tokens.get(i + 2).map(|t| &t.kind), Some(TokenKind::LBrace));
        if is_def {
            let (name, def, next) = parse_definition(tokens, i)?;
            if let Some(prev) = defs.get(&name) {
                return Err(err(
                    tok.span.clone(),
                    format!("macro `{}` is already defined at {}", name, prev.span),
                ));
            }
            defs.insert(name, def);
            i = next;
            // Skip a single trailing separator left by the definition so the
            // stripped stream does not accumulate stray blank lines.
            while matches!(
                tokens.get(i).map(|t| &t.kind),
                Some(TokenKind::Newline) | Some(TokenKind::Semi)
            ) {
                i += 1;
            }
        } else {
            body.push(tok.clone());
            i += 1;
        }
    }
    Ok((defs, body))
}

/// Parse one `macro name { rules }` definition starting at `start` (the
/// `macro` token). Returns the name, the definition, and the index just past
/// the closing brace.
fn parse_definition(
    tokens: &[Token],
    start: usize,
) -> Result<(String, MacroDef, usize), RavenError> {
    let kw_span = tokens[start].span.clone();
    let name = match &tokens[start + 1].kind {
        TokenKind::Identifier(s) => s.clone(),
        _ => unreachable!("checked by caller"),
    };
    // tokens[start + 2] is the opening brace of the macro body.
    let body_open = start + 2;
    let body_close = matching_close(tokens, body_open).ok_or_else(|| {
        err(
            kw_span.clone(),
            format!("macro `{}` body is not closed", name),
        )
    })?;

    let inner = &tokens[body_open + 1..body_close];
    let rules = parse_rules(inner, &name, &kw_span)?;
    if rules.is_empty() {
        return Err(err(
            kw_span.clone(),
            format!("macro `{}` has no rules", name),
        ));
    }
    Ok((
        name,
        MacroDef {
            rules,
            span: kw_span,
        },
        body_close + 1,
    ))
}

/// Parse the `(matcher) => { template }` arms inside a macro body.
fn parse_rules(inner: &[Token], name: &str, span: &Span) -> Result<Vec<Rule>, RavenError> {
    let mut rules = Vec::new();
    let mut i = 0;
    while i < inner.len() {
        // Skip separators between arms.
        if matches!(
            inner[i].kind,
            TokenKind::Newline | TokenKind::Semi | TokenKind::Comma
        ) {
            i += 1;
            continue;
        }
        if !matches!(inner[i].kind, TokenKind::LParen) {
            return Err(err(
                inner[i].span.clone(),
                format!("macro `{}`: expected `(` to start a rule matcher", name),
            ));
        }
        let mclose = matching_close(inner, i).ok_or_else(|| {
            err(
                inner[i].span.clone(),
                format!("macro `{}`: matcher `(` is not closed", name),
            )
        })?;
        let matcher = parse_matcher(&inner[i + 1..mclose], name)?;
        let mut j = mclose + 1;
        skip_newlines(inner, &mut j);
        if !matches!(inner.get(j).map(|t| &t.kind), Some(TokenKind::FatArrow)) {
            return Err(err(
                inner
                    .get(j)
                    .map(|t| t.span.clone())
                    .unwrap_or_else(|| span.clone()),
                format!("macro `{}`: expected `=>` after rule matcher", name),
            ));
        }
        j += 1;
        skip_newlines(inner, &mut j);
        if !matches!(inner.get(j).map(|t| &t.kind), Some(TokenKind::LBrace)) {
            return Err(err(
                inner
                    .get(j)
                    .map(|t| t.span.clone())
                    .unwrap_or_else(|| span.clone()),
                format!("macro `{}`: expected `{{` to start a rule template", name),
            ));
        }
        let tclose = matching_close(inner, j).ok_or_else(|| {
            err(
                inner[j].span.clone(),
                format!("macro `{}`: template `{{` is not closed", name),
            )
        })?;
        let template = parse_template(&inner[j + 1..tclose]);
        rules.push(Rule { matcher, template });
        i = tclose + 1;
    }
    Ok(rules)
}

/// Parse a matcher token slice into match items.
fn parse_matcher(slice: &[Token], name: &str) -> Result<Vec<MatchItem>, RavenError> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < slice.len() {
        match &slice[i].kind {
            TokenKind::Newline => {
                i += 1;
            }
            TokenKind::Dollar => {
                let var = match slice.get(i + 1).map(|t| &t.kind) {
                    Some(TokenKind::Identifier(s)) => s.clone(),
                    _ => {
                        return Err(err(
                            slice[i].span.clone(),
                            format!(
                                "macro `{}`: `$` must be followed by a metavariable name",
                                name
                            ),
                        ))
                    }
                };
                if !matches!(slice.get(i + 2).map(|t| &t.kind), Some(TokenKind::Colon)) {
                    return Err(err(
                        slice[i].span.clone(),
                        format!(
                            "macro `{}`: metavariable `${}` needs a fragment, e.g. `${}:expr`",
                            name, var, var
                        ),
                    ));
                }
                let frag = match slice.get(i + 3).map(|t| &t.kind) {
                    Some(TokenKind::Identifier(s)) if s == "expr" => Fragment::Expr,
                    Some(TokenKind::Identifier(s)) if s == "ident" => Fragment::Ident,
                    other => {
                        return Err(err(
                            slice[i].span.clone(),
                            format!(
                                "macro `{}`: unsupported fragment `{}` (this slice supports `expr` and `ident`)",
                                name,
                                other.map(describe).unwrap_or_else(|| "?".into())
                            ),
                        ))
                    }
                };
                items.push(MatchItem::Meta { name: var, frag });
                i += 4;
            }
            _ => {
                items.push(MatchItem::Literal(slice[i].kind.clone()));
                i += 1;
            }
        }
    }
    Ok(items)
}

/// Parse a template token slice into template items. Newlines are dropped so
/// templates can span lines without injecting separators into expressions.
fn parse_template(slice: &[Token]) -> Vec<TemplateItem> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < slice.len() {
        match &slice[i].kind {
            TokenKind::Newline => {
                i += 1;
            }
            TokenKind::Dollar => {
                if let Some(TokenKind::Identifier(s)) = slice.get(i + 1).map(|t| &t.kind) {
                    items.push(TemplateItem::Meta(s.clone()));
                    i += 2;
                } else {
                    items.push(TemplateItem::Token(slice[i].clone()));
                    i += 1;
                }
            }
            _ => {
                items.push(TemplateItem::Token(slice[i].clone()));
                i += 1;
            }
        }
    }
    items
}

/// True when `stream` still contains any `name!(` macro-call shape. Unknown
/// names are included so `expand_once` can report them rather than leaving an
/// unexpandable call for the parser.
fn contains_call(stream: &[Token]) -> bool {
    (0..stream.len()).any(|i| call_name_at(stream, i).is_some())
}

/// If a macro-call shape `<ident> ! (` begins at `i`, return the macro name.
fn call_name_at(stream: &[Token], i: usize) -> Option<String> {
    let TokenKind::Identifier(name) = &stream.get(i)?.kind else {
        return None;
    };
    if !matches!(stream.get(i + 1).map(|t| &t.kind), Some(TokenKind::Bang)) {
        return None;
    }
    if !matches!(stream.get(i + 2).map(|t| &t.kind), Some(TokenKind::LParen)) {
        return None;
    }
    Some(name.clone())
}

/// Rewrite every outermost macro call in `stream` once.
fn expand_once(
    stream: &[Token],
    defs: &HashMap<String, MacroDef>,
    spans: &mut SpanGen,
) -> Result<Vec<Token>, RavenError> {
    let mut out = Vec::with_capacity(stream.len());
    let mut i = 0;
    while i < stream.len() {
        if let Some(name) = call_name_at(stream, i) {
            if let Some(def) = defs.get(&name) {
                let lparen = i + 2;
                let rparen = matching_close(stream, lparen).ok_or_else(|| {
                    err(
                        stream[lparen].span.clone(),
                        format!("macro `{}!`: `(` is not closed", name),
                    )
                })?;
                let args = &stream[lparen + 1..rparen];
                let call_span = stream[i].span.clone();
                let expanded = expand_call(&name, def, args, &call_span, spans)?;
                out.extend(expanded);
                i = rparen + 1;
                continue;
            } else {
                return Err(err(
                    stream[i].span.clone(),
                    format!("unknown macro `{}!`", name),
                ));
            }
        }
        out.push(stream[i].clone());
        i += 1;
    }
    Ok(out)
}

/// Match `args` against the rules of `def`, then instantiate the matching
/// template. The first rule that matches wins.
fn expand_call(
    name: &str,
    def: &MacroDef,
    args: &[Token],
    call_span: &Span,
    spans: &mut SpanGen,
) -> Result<Vec<Token>, RavenError> {
    for rule in &def.rules {
        if let Some(binds) = try_match(&rule.matcher, args) {
            return Ok(instantiate(&rule.template, &binds, call_span, spans));
        }
    }
    Err(err(
        call_span.clone(),
        format!("no rule of macro `{}!` matches the given arguments", name),
    ))
}

/// Try to match an argument token run against one matcher. Returns the
/// metavariable bindings on success.
fn try_match(matcher: &[MatchItem], args: &[Token]) -> Option<HashMap<String, Vec<Token>>> {
    let args = strip_newlines(args);
    let mut binds: HashMap<String, Vec<Token>> = HashMap::new();
    let mut pos = 0;
    for (idx, item) in matcher.iter().enumerate() {
        match item {
            MatchItem::Literal(kind) => {
                let tok = args.get(pos)?;
                if !same_kind(&tok.kind, kind) {
                    return None;
                }
                pos += 1;
            }
            MatchItem::Meta { name, frag } => match frag {
                Fragment::Ident => {
                    let tok = args.get(pos)?;
                    if !matches!(tok.kind, TokenKind::Identifier(_)) {
                        return None;
                    }
                    binds.insert(name.clone(), vec![tok.clone()]);
                    pos += 1;
                }
                Fragment::Expr => {
                    let delim = next_literal(matcher, idx + 1);
                    let end = capture_balanced(&args, pos, delim.as_ref())?;
                    if end == pos {
                        return None;
                    }
                    binds.insert(name.clone(), args[pos..end].to_vec());
                    pos = end;
                }
            },
        }
    }
    if pos != args.len() {
        return None;
    }
    Some(binds)
}

/// The next literal token kind in the matcher at or after `from`, used as the
/// stop delimiter for an `expr` capture.
fn next_literal(matcher: &[MatchItem], from: usize) -> Option<TokenKind> {
    matcher.iter().skip(from).find_map(|m| match m {
        MatchItem::Literal(k) => Some(k.clone()),
        MatchItem::Meta { .. } => None,
    })
}

/// Capture a balanced token run from `start` until a top-level `delim` (or
/// the end of `args` when `delim` is `None`). Bracket depth must be balanced
/// at the stop point. Returns the index of the stop position.
fn capture_balanced(args: &[Token], start: usize, delim: Option<&TokenKind>) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut i = start;
    while i < args.len() {
        let k = &args[i].kind;
        match k {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
        if depth == 0 {
            if let Some(d) = delim {
                if same_kind(k, d) {
                    break;
                }
            }
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    Some(i)
}

/// Instantiate a template with the given bindings. Spliced and verbatim
/// tokens all carry the call span so diagnostics point at the call site.
fn instantiate(
    template: &[TemplateItem],
    binds: &HashMap<String, Vec<Token>>,
    call_span: &Span,
    spans: &mut SpanGen,
) -> Vec<Token> {
    let mut out = Vec::new();
    for item in template {
        match item {
            TemplateItem::Meta(name) => {
                if let Some(toks) = binds.get(name) {
                    for t in toks {
                        out.push(Token::new(t.kind.clone(), spans.fresh(call_span)));
                    }
                }
            }
            TemplateItem::Token(t) => out.push(Token::new(t.kind.clone(), spans.fresh(call_span))),
        }
    }
    out
}

/// Find the matching close bracket for the open bracket at `open`. Supports
/// `(`/`)`, `[`/`]`, and `{`/`}`. Returns the index of the matching close.
fn matching_close(tokens: &[Token], open: usize) -> Option<usize> {
    let close = match tokens.get(open)?.kind {
        TokenKind::LParen => TokenKind::RParen,
        TokenKind::LBracket => TokenKind::RBracket,
        TokenKind::LBrace => TokenKind::RBrace,
        _ => return None,
    };
    let open_kind = tokens[open].kind.clone();
    let mut depth = 0i32;
    let mut i = open;
    while i < tokens.len() {
        let k = &tokens[i].kind;
        if same_kind(k, &open_kind) {
            depth += 1;
        } else if same_kind(k, &close) {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn strip_newlines(tokens: &[Token]) -> Vec<Token> {
    tokens
        .iter()
        .filter(|t| !matches!(t.kind, TokenKind::Newline))
        .cloned()
        .collect()
}

fn skip_newlines(tokens: &[Token], i: &mut usize) {
    while matches!(tokens.get(*i).map(|t| &t.kind), Some(TokenKind::Newline)) {
        *i += 1;
    }
}

/// Compare token kinds by discriminant, treating any two identifiers (or any
/// two of a payload-carrying kind) as the same kind. Matcher literals match
/// on kind, not on the literal payload.
fn same_kind(a: &TokenKind, b: &TokenKind) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

fn describe(kind: &TokenKind) -> String {
    crate::parser::describe_token(kind)
}

/// Build a macro-expansion error. These surface during the pre-parse pass,
/// so they reuse the parser error channel with a custom message.
fn err(span: Span, msg: String) -> RavenError {
    RavenError::parse(ParseError::Custom(msg), span)
}

#[cfg(test)]
mod tests;
