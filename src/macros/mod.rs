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
//! * Repetition: `$( <sub> )<sep>*` (zero or more) and `$( <sub> )<sep>+`
//!   (one or more) in both matchers and templates, with an optional single
//!   separator token. Metavariables under a matcher repetition bind to a
//!   sequence and must be used under a template repetition. Repetition nests
//!   to any depth: a group inside another binds a sequence of sequences.
//! * Hygiene: a `let`/`const`/`for` target or a function or closure parameter a
//!   template introduces is renamed to a fresh, unique name per expansion, so a
//!   template temporary cannot capture or be captured by a caller name. The
//!   `let`/`const`/`for` rename is scope-aware: a binding only renames its own
//!   lexical scope, so a nested local does not rename a free use in an outer
//!   scope. A match-arm pattern variable is kept verbatim (a shorthand struct
//!   field is also the field name) but recorded so its uses resolve. A free
//!   identifier a template names (a function it calls) is marked so the resolver
//!   resolves it at the macro's definition site (the module scope), not the call
//!   site. Metavariable-captured tokens keep their original identity, referring
//!   to call-site bindings.
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

/// Upper bound on the token-stream length during expansion. A macro that
/// expands one call into several (`boom!() => boom!() + boom!()`) doubles the
/// stream every pass and exhausts memory long before the pass limit, so a size
/// cap reports the same likely-recursive diagnostic instead of aborting on a
/// failed allocation. Far above any real program's expanded token count.
const TOKEN_LIMIT: usize = 2_000_000;

/// The free (definition-site) identifiers a macro expansion introduces, keyed
/// by file and start offset. The resolver resolves these against the module
/// scope so a call-site local cannot capture them. The file is part of the key
/// so a synthetic offset never collides with the same offset in another file
/// (the bundled stdlib is resolved in the same compilation).
pub type DefSites = std::collections::HashSet<(std::sync::Arc<std::path::PathBuf>, usize)>;

/// A single metavariable fragment kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fragment {
    /// A balanced expression token run, up to the next matcher delimiter.
    Expr,
    /// A single identifier token.
    Ident,
    /// A type, captured as a balanced token run like `expr` (so `List<Int>`
    /// with its angle brackets is one fragment).
    Ty,
    /// A single literal token (number, string, char, or boolean).
    Literal,
    /// A pattern, captured as a balanced token run like `expr`.
    Pat,
    /// A brace-delimited block `{ ... }`, captured with its braces.
    Block,
}

/// One element of a matcher: a bound metavariable, a literal token that must
/// appear verbatim, or a repetition group.
#[derive(Debug, Clone)]
enum MatchItem {
    Meta {
        name: String,
        frag: Fragment,
    },
    Literal(TokenKind),
    /// `$( <sub> )<sep>*` or `...+`. `plus` is true for `+` (one or more).
    Rep {
        sub: Vec<MatchItem>,
        sep: Option<TokenKind>,
        plus: bool,
    },
}

/// One template element: a metavariable splice, a verbatim token, or a
/// repetition group expanded once per captured repetition.
#[derive(Debug, Clone)]
enum TemplateItem {
    Meta(String),
    Token(Token),
    /// `$( <sub> )<sep>*` (or `+`); the trailing marker is ignored at
    /// instantiation, the count comes from the matched sequence length.
    Rep {
        sub: Vec<TemplateItem>,
        sep: Option<Token>,
    },
}

/// A captured metavariable: either a single token run, or a sequence of
/// captures (one per repetition) when captured under a matcher repetition. The
/// sequence holds `Capture`s rather than token runs so a metavariable under
/// two nested repetitions binds to a sequence of sequences.
#[derive(Debug, Clone)]
enum Capture {
    Single(Vec<Token>),
    Seq(Vec<Capture>),
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

/// A file's collected macro definitions, kept so macro calls that surface
/// after the main token pre-pass (string-interpolation fragments are
/// re-lexed during parsing) can still be expanded. Empty when the file
/// defines no macros.
#[derive(Debug, Clone, Default)]
pub struct MacroTable {
    defs: HashMap<String, MacroDef>,
}

impl MacroTable {
    /// True when the file defines no macros, so expansion is a no-op.
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// Collect a file's macro definitions into a reusable table without
/// expanding anything. Run on the original token stream (definitions still
/// present); the result feeds [`expand_with_table`] for snippets re-lexed
/// during parsing.
pub fn collect_macro_table(tokens: &[Token]) -> Result<MacroTable, RavenError> {
    if !has_macro_keyword(tokens) {
        return Ok(MacroTable::default());
    }
    let (defs, _body) = collect_defs(tokens)?;
    Ok(MacroTable { defs })
}

/// Expand macro calls in `tokens` using a previously collected [`MacroTable`].
///
/// Used for string-interpolation fragments, which are lexed during parsing,
/// after the file's main macro pre-pass has already run and stripped the
/// definitions from the stream. Returns the input unchanged when the table
/// is empty or the tokens contain no macro call.
pub fn expand_with_table(tokens: &[Token], table: &MacroTable) -> Result<Vec<Token>, RavenError> {
    if table.is_empty() || !contains_call(tokens) {
        return Ok(tokens.to_vec());
    }
    let mut stream = tokens.to_vec();
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
        stream = expand_once(&stream, &table.defs, &mut spans)?;
        if stream.len() > TOKEN_LIMIT {
            return Err(token_limit_error(&stream, tokens));
        }
    }
    Ok(stream)
}

/// The "likely recursive" diagnostic for an expansion whose token stream grew
/// past [`TOKEN_LIMIT`]. Anchored at the first token of the runaway stream, or
/// the original input's first token when the stream is somehow empty.
fn token_limit_error(stream: &[Token], original: &[Token]) -> RavenError {
    let span = stream
        .first()
        .or_else(|| original.first())
        .map(|t| t.span.clone())
        .expect("a non-empty token stream");
    err(
        span,
        format!(
            "macro expansion produced over {} tokens; a macro is likely recursive",
            TOKEN_LIMIT
        ),
    )
}

/// Expand all declarative macros in `tokens`.
///
/// Returns the rewritten token stream (still ending in `Eof`). When the
/// source defines no macros the input is returned unchanged.
pub fn expand_tokens(tokens: &[Token]) -> Result<Vec<Token>, RavenError> {
    Ok(expand_tokens_hygienic(tokens)?.0)
}

/// Like [`expand_tokens`], but also returns the start offsets of the
/// definition-site (free) identifiers the expansion introduced. The driver
/// hands the set to the resolver, which resolves those identifiers against the
/// module scope so a call-site local cannot capture them (referential
/// hygiene). The set is empty when the file defines no macros.
pub fn expand_tokens_hygienic(tokens: &[Token]) -> Result<(Vec<Token>, DefSites), RavenError> {
    if !has_macro_keyword(tokens) {
        return Ok((tokens.to_vec(), DefSites::new()));
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
        if stream.len() > TOKEN_LIMIT {
            return Err(token_limit_error(&stream, tokens));
        }
    }
    Ok((stream, spans.def_sites))
}

/// Allocator of unique synthetic byte ranges for expanded tokens. The
/// `line`/`col` of the originating call site are preserved for diagnostics;
/// only the byte range is made unique so use-site keys stay distinct.
struct SpanGen {
    next: usize,
    /// The free (non-metavariable, non-renamed) identifiers a template
    /// introduces, by file and start offset (see [`DefSites`]).
    def_sites: DefSites,
}

impl SpanGen {
    fn starting_after(tokens: &[Token]) -> Self {
        let max_end = tokens.iter().map(|t| t.span.end).max().unwrap_or(0);
        SpanGen {
            next: max_end.saturating_add(1),
            def_sites: DefSites::new(),
        }
    }

    /// A fresh one-byte span that borrows the file, line, and column of
    /// `at` but occupies a byte range used by nothing else.
    fn fresh(&mut self, at: &Span) -> Span {
        let start = self.next;
        self.next = self.next.saturating_add(1);
        Span::new(at.file.clone(), start, start + 1, at.line, at.col)
    }

    /// A fresh identifier name derived from `base`, unique across the whole
    /// expansion. The `$` keeps it out of the source identifier space so it
    /// can never clash with a user-written name.
    fn gensym(&mut self, base: &str) -> String {
        let n = self.next;
        self.next = self.next.saturating_add(1);
        format!("{}${}", base, n)
    }
}

/// True when an item-position `macro` keyword appears. `macro` is a
/// contextual identifier, so we only treat it as the keyword when it begins
/// a definition shape (`macro <ident> {`).
/// Whether `tokens` declare any macro. Because macros are file-local (a
/// `name!(...)` call requires its `macro name { ... }` definition in the
/// same file), this also tells whether the file uses macros at all. The
/// formatter uses it to leave macro-using files untouched, since macro
/// definitions and invocations have no AST representation to format.
pub fn contains_macros(tokens: &[Token]) -> bool {
    has_macro_keyword(tokens)
}

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
        let template = parse_template(&inner[j + 1..tclose], name)?;
        // Every metavariable the template splices must be bound by the matcher.
        // An undefined one (a typo, say) was silently dropped at expansion;
        // reject it at the definition instead.
        let bound = matcher_meta_names(&matcher);
        // A matcher must not bind the same metavariable name twice: the second
        // capture silently overwrote the first, so `$x` referred to only one of
        // the arguments.
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for var in &bound {
            if !seen.insert(var.as_str()) {
                return Err(err(
                    inner[i].span.clone(),
                    format!(
                        "macro `{}`: metavariable `${}` is bound more than once",
                        name, var
                    ),
                ));
            }
        }
        for var in template_meta_names(&template) {
            if !bound.contains(&var) {
                return Err(err(
                    inner[j].span.clone(),
                    format!(
                        "macro `{}`: template uses undefined metavariable `${}`",
                        name, var
                    ),
                ));
            }
        }
        rules.push(Rule { matcher, template });
        i = tclose + 1;
    }
    Ok(rules)
}

/// Names of metavariables a matcher binds, including those inside repetitions.
fn matcher_meta_names(items: &[MatchItem]) -> Vec<String> {
    let mut out = Vec::new();
    for it in items {
        match it {
            MatchItem::Meta { name, .. } => out.push(name.clone()),
            MatchItem::Rep { sub, .. } => out.extend(matcher_meta_names(sub)),
            MatchItem::Literal(_) => {}
        }
    }
    out
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
            TokenKind::Dollar
                if matches!(slice.get(i + 1).map(|t| &t.kind), Some(TokenKind::LParen)) =>
            {
                let open = i + 1;
                let close = matching_close(slice, open).ok_or_else(|| {
                    err(
                        slice[i].span.clone(),
                        format!("macro `{}`: repetition `$(` is not closed", name),
                    )
                })?;
                let sub = parse_matcher(&slice[open + 1..close], name)?;
                let (sep, plus, next) = parse_rep_suffix(slice, close + 1, name)?;
                items.push(MatchItem::Rep { sub, sep, plus });
                i = next;
            }
            TokenKind::Dollar => {
                let var = match slice.get(i + 1).map(|t| &t.kind) {
                    Some(TokenKind::Identifier(s)) => s.clone(),
                    _ => {
                        return Err(err(
                            slice[i].span.clone(),
                            format!(
                                "macro `{}`: `$` must be followed by a metavariable name or `(`",
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
                    Some(TokenKind::Identifier(s)) if s == "ty" => Fragment::Ty,
                    Some(TokenKind::Identifier(s)) if s == "literal" => Fragment::Literal,
                    Some(TokenKind::Identifier(s)) if s == "pat" => Fragment::Pat,
                    Some(TokenKind::Identifier(s)) if s == "block" => Fragment::Block,
                    other => {
                        return Err(err(
                            slice[i].span.clone(),
                            format!(
                                "macro `{}`: unsupported fragment `{}` (supported: expr, ident, ty, literal, pat, block)",
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

/// Parse the suffix of a `$(...)` group: an optional single separator token
/// followed by `*` or `+`. Returns the separator, whether it was `+`, and the
/// index just past the marker.
fn parse_rep_suffix(
    slice: &[Token],
    after_close: usize,
    name: &str,
) -> Result<(Option<TokenKind>, bool, usize), RavenError> {
    let mut i = after_close;
    let span = slice[after_close - 1].span.clone();
    // The marker is `*` or `+`. Anything before it (and not `*`/`+`) is the
    // single optional separator token.
    let sep = match slice.get(i).map(|t| &t.kind) {
        Some(TokenKind::Star) | Some(TokenKind::Plus) | None => None,
        Some(other) => {
            let s = other.clone();
            i += 1;
            Some(s)
        }
    };
    match slice.get(i).map(|t| &t.kind) {
        Some(TokenKind::Star) => Ok((sep, false, i + 1)),
        Some(TokenKind::Plus) => Ok((sep, true, i + 1)),
        _ => Err(err(
            span,
            format!(
                "macro `{}`: repetition `$( ... )` must end with `*` or `+`",
                name
            ),
        )),
    }
}

/// Parse a template token slice into template items. Newlines are dropped so
/// templates can span lines without injecting separators into expressions.
fn parse_template(slice: &[Token], name: &str) -> Result<Vec<TemplateItem>, RavenError> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < slice.len() {
        match &slice[i].kind {
            TokenKind::Newline => {
                i += 1;
            }
            TokenKind::Dollar
                if matches!(slice.get(i + 1).map(|t| &t.kind), Some(TokenKind::LParen)) =>
            {
                let open = i + 1;
                let close = matching_close(slice, open).ok_or_else(|| {
                    err(
                        slice[i].span.clone(),
                        format!("macro `{}`: template repetition `$(` is not closed", name),
                    )
                })?;
                let sub = parse_template(&slice[open + 1..close], name)?;
                // A separator may sit between `)` and the `*`/`+` marker; the
                // marker itself is dropped, only the separator is kept.
                let sep = match slice.get(close + 1).map(|t| &t.kind) {
                    Some(TokenKind::Star) | Some(TokenKind::Plus) | None => None,
                    Some(_) => Some(slice[close + 1].clone()),
                };
                let marker = if sep.is_some() { close + 2 } else { close + 1 };
                match slice.get(marker).map(|t| &t.kind) {
                    Some(TokenKind::Star) | Some(TokenKind::Plus) => {}
                    _ => {
                        return Err(err(
                            slice[i].span.clone(),
                            format!(
                            "macro `{}`: template repetition `$( ... )` must end with `*` or `+`",
                            name
                        ),
                        ))
                    }
                }
                items.push(TemplateItem::Rep { sub, sep });
                i = marker + 1;
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
    Ok(items)
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
fn try_match(matcher: &[MatchItem], args: &[Token]) -> Option<HashMap<String, Capture>> {
    let args = strip_newlines(args);
    let mut binds: HashMap<String, Capture> = HashMap::new();
    let pos = match_seq(matcher, &args, 0, None, None, &mut binds)?;
    if pos != args.len() {
        return None;
    }
    Some(binds)
}

/// Match the items of `matcher` sequentially against `args` starting at `pos`,
/// recording captures in `binds`. `outer_delim` is the stop token for a
/// trailing `expr` when this matcher is the body of a repetition (the rep
/// separator), `None` at top level. Returns the position after the match.
fn match_seq(
    matcher: &[MatchItem],
    args: &[Token],
    mut pos: usize,
    outer_delim: Option<&TokenKind>,
    // The token that follows the repetition this sequence is the body of, if
    // any. The last element captured here stops before it, so a repetition can
    // be followed by another matcher item (`$($x:expr),* ; $tail`).
    follow: Option<&TokenKind>,
    binds: &mut HashMap<String, Capture>,
) -> Option<usize> {
    for (idx, item) in matcher.iter().enumerate() {
        match item {
            MatchItem::Literal(kind) => {
                // A literal matcher token must appear verbatim: compare the full
                // token, not just its kind, so `(foo)` matches the identifier
                // `foo` and not every identifier, and `(1)` matches only `1`.
                let tok = args.get(pos)?;
                if &tok.kind != kind {
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
                    binds.insert(name.clone(), Capture::Single(vec![tok.clone()]));
                    pos += 1;
                }
                Fragment::Literal => {
                    let tok = args.get(pos)?;
                    if !is_literal_token(&tok.kind) {
                        return None;
                    }
                    binds.insert(name.clone(), Capture::Single(vec![tok.clone()]));
                    pos += 1;
                }
                Fragment::Block => {
                    let end = capture_block(args, pos)?;
                    binds.insert(name.clone(), Capture::Single(args[pos..end].to_vec()));
                    pos = end;
                }
                // A type and a pattern capture a balanced token run, the same
                // way an expression does: the angle brackets of `List<Int>`
                // and the parentheses of a constructor pattern stay balanced.
                Fragment::Expr | Fragment::Ty | Fragment::Pat => {
                    let delim = next_delim(matcher, idx + 1, outer_delim);
                    // A `ty` fragment also balances angle brackets, so a comma
                    // inside generic arguments (`Pair<Int, String>`) does not end
                    // the type. `<`/`>` are comparison operators in an `expr` or
                    // `pat`, so those fragments leave angles untracked.
                    let angles = matches!(frag, Fragment::Ty);
                    // The last item of a repetition body also stops at the token
                    // following the repetition (`follow`), only relevant when
                    // `delim` (the next matcher delimiter) is not hit first.
                    let end = capture_balanced(args, pos, delim.as_ref(), follow, angles)?;
                    if end == pos {
                        return None;
                    }
                    binds.insert(name.clone(), Capture::Single(args[pos..end].to_vec()));
                    pos = end;
                }
            },
            MatchItem::Rep { sub, sep, plus } => {
                // The token after the repetition delimits the repetition's last
                // element, so it can be followed by another matcher item.
                let rep_follow = next_delim(matcher, idx + 1, outer_delim);
                pos = match_rep(
                    sub,
                    sep.as_ref(),
                    *plus,
                    args,
                    pos,
                    rep_follow.as_ref(),
                    binds,
                )?;
            }
        }
    }
    Some(pos)
}

/// Match a repetition group: zero or more (`*`) or one or more (`+`) copies of
/// `sub`, separated by `sep`. Every metavariable declared in `sub` is recorded
/// as a `Capture::Seq`, one entry per iteration (an empty sequence on zero
/// reps), so the matching template repetition knows the count.
fn match_rep(
    sub: &[MatchItem],
    sep: Option<&TokenKind>,
    plus: bool,
    args: &[Token],
    mut pos: usize,
    // The token following the repetition in the enclosing matcher, used to
    // bound the last captured element so the repetition can be followed by
    // another matcher item.
    follow: Option<&TokenKind>,
    binds: &mut HashMap<String, Capture>,
) -> Option<usize> {
    let names = meta_names(sub);
    let mut seqs: HashMap<String, Vec<Capture>> = HashMap::new();
    for n in &names {
        seqs.insert(n.clone(), Vec::new());
    }
    let mut count = 0usize;
    loop {
        if pos >= args.len() {
            break;
        }
        // Between iterations, consume the separator. If a separator is defined
        // and not present, the repetition stops.
        if count > 0 {
            if let Some(s) = sep {
                // Compare the full token: a separator that carries a payload
                // must match that exact token, not every token of its kind.
                if !args.get(pos).map(|t| &t.kind == s).unwrap_or(false) {
                    break;
                }
                pos += 1;
            }
        }
        let mut iter_binds: HashMap<String, Capture> = HashMap::new();
        match match_seq(sub, args, pos, sep, follow, &mut iter_binds) {
            Some(next) if next > pos => {
                // Each iteration contributes one capture per metavariable. A
                // metavariable inside a nested repetition contributes a
                // `Seq`; a direct one contributes a `Single`.
                for n in &names {
                    let cap = iter_binds
                        .get(n)
                        .cloned()
                        .unwrap_or_else(|| Capture::Single(Vec::new()));
                    seqs.get_mut(n).unwrap().push(cap);
                }
                pos = next;
                count += 1;
            }
            _ => {
                // The separator was consumed but no iteration followed: reject
                // a trailing separator with nothing after it.
                if count > 0 && sep.is_some() {
                    return None;
                }
                break;
            }
        }
    }
    if plus && count == 0 {
        return None;
    }
    for (n, seq) in seqs {
        binds.insert(n, Capture::Seq(seq));
    }
    Some(pos)
}

/// The names of all metavariables declared in `items`, recursing into nested
/// repetition groups so an outer repetition tracks the captures of an inner
/// one as well.
fn meta_names(items: &[MatchItem]) -> Vec<String> {
    let mut out = Vec::new();
    for it in items {
        match it {
            MatchItem::Meta { name, .. } => out.push(name.clone()),
            MatchItem::Rep { sub, .. } => out.extend(meta_names(sub)),
            MatchItem::Literal(_) => {}
        }
    }
    out
}

/// The stop delimiter for an `expr` capture: the next literal token kind in
/// `matcher` at or after `from`, falling back to `outer_delim` (the enclosing
/// repetition separator) when no literal follows.
fn next_delim(
    matcher: &[MatchItem],
    from: usize,
    outer_delim: Option<&TokenKind>,
) -> Option<TokenKind> {
    matcher
        .iter()
        .skip(from)
        .find_map(|m| match m {
            MatchItem::Literal(k) => Some(k.clone()),
            _ => None,
        })
        .or_else(|| outer_delim.cloned())
}

/// Capture a balanced token run from `start` until a top-level `delim` or
/// `delim2` (or the end of `args` when both are `None`). `delim2` is the token
/// that follows the enclosing repetition, if any, so the last element of a
/// repetition stops before it (`$($x:expr),* ; $tail` captures the final `$x`
/// up to the `;`). Bracket depth must be balanced at the stop point. Returns
/// the index of the stop position.
fn capture_balanced(
    args: &[Token],
    start: usize,
    delim: Option<&TokenKind>,
    delim2: Option<&TokenKind>,
    angles: bool,
) -> Option<usize> {
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
            // For a `ty` fragment, angle brackets are generic delimiters: `<`
            // opens one and `>` closes it, and a `<<`/`>>` token spans two (a
            // nested generic like `Vec<Vec<Int>>`).
            TokenKind::Lt if angles => depth += 1,
            TokenKind::Shl if angles => depth += 2,
            TokenKind::Gt if angles && depth > 0 => depth -= 1,
            TokenKind::Shr if angles && depth > 0 => depth = (depth - 2).max(0),
            _ => {}
        }
        // Compare the full token, not just its kind: a delimiter that carries a
        // payload (a literal identifier or number, for instance a follow token
        // like `foo`) must stop the capture only on that exact token, not on
        // every identifier or number.
        if depth == 0 && (delim.is_some_and(|d| k == d) || delim2.is_some_and(|d| k == d)) {
            break;
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    Some(i)
}

/// Whether `kind` is a literal token (the `literal` fragment matches one).
fn is_literal_token(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::IntLit(_)
            | TokenKind::FloatLit(_)
            | TokenKind::StringLit(_)
            | TokenKind::BlockStringLit(_)
            | TokenKind::CharLit(_)
            | TokenKind::CStringLit(_)
            | TokenKind::True
            | TokenKind::False
    )
}

/// Capture a brace-delimited block `{ ... }` starting at `start`, returning
/// the index just past its closing `}`. Returns `None` when `start` is not a
/// `{` or the braces are unbalanced.
fn capture_block(args: &[Token], start: usize) -> Option<usize> {
    if !matches!(args.get(start)?.kind, TokenKind::LBrace) {
        return None;
    }
    let mut depth = 0usize;
    let mut i = start;
    while i < args.len() {
        match args[i].kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Instantiate a template with the given bindings. Spliced and verbatim
/// tokens all carry the call span so diagnostics point at the call site.
///
/// Basic hygiene: identifiers introduced by the template at a binding site
/// (`let`/`const`/`for` targets) are renamed to fresh, unique names for this
/// expansion, and every verbatim use of the same spelling in the template is
/// renamed to match. Metavariable splices keep their captured spelling, so
/// they still refer to caller bindings.
fn instantiate(
    template: &[TemplateItem],
    binds: &HashMap<String, Capture>,
    call_span: &Span,
    spans: &mut SpanGen,
) -> Vec<Token> {
    // Instantiate first (splice metavariables, expand repetitions) into a flat
    // token stream, tracking which tokens came from the template versus a
    // captured metavariable. Then apply hygiene as a scope-aware pass over that
    // stream: a template-introduced binding renames only its own lexical scope,
    // so a nested local does not rename a free use in an outer scope.
    let mut out = Vec::new();
    let mut from_template = Vec::new();
    instantiate_into(
        template,
        binds,
        call_span,
        spans,
        &mut out,
        &mut from_template,
    );
    scope_aware_rename(&mut out, &from_template, spans);
    out
}

/// How a template-introduced binding is made hygienic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindKind {
    /// A `let`/`const`/`for` target: gensym'd and block-scoped, so it only
    /// renames its own `{}` scope and nested ones.
    Block,
    /// A function or closure parameter: gensym'd and renamed across the whole
    /// expansion (it is not brace-delimited). Safe because a parameter name is
    /// positional, never a struct field label.
    Param,
    /// A match-arm pattern variable: kept verbatim (not gensym'd) but recorded
    /// so its uses resolve rather than being treated as free. A shorthand struct
    /// pattern field (`Point { x }`) is both the field name and the binding, so
    /// renaming it would break the field match.
    Pattern,
}

/// The token indices of the template-introduced bindings in an instantiated
/// stream, each mapped to its spelling and how it should be made hygienic. Only
/// tokens that came from the template (`from_template[i]`) are considered: a
/// captured metavariable keeps its caller identity and is never a template
/// binding.
fn collect_binding_indices(
    tokens: &[Token],
    from_template: &[bool],
) -> HashMap<usize, (String, BindKind)> {
    let mut bindings = HashMap::new();
    let is_t = |i: usize| from_template.get(i).copied().unwrap_or(false);
    let mut i = 0;
    while i < tokens.len() {
        if is_t(i) {
            match &tokens[i].kind {
                TokenKind::Let | TokenKind::Const | TokenKind::For => {
                    if is_t(i + 1) {
                        if let Some(TokenKind::Identifier(s)) = tokens.get(i + 1).map(|t| &t.kind) {
                            bindings.insert(i + 1, (s.clone(), BindKind::Block));
                        }
                    }
                }
                // Scan forward to the parameter list's `(`, past an optional
                // function name (a literal identifier or a spliced metavariable).
                TokenKind::Fun => {
                    let mut j = i + 1;
                    while j < tokens.len() {
                        if is_t(j) && matches!(tokens[j].kind, TokenKind::LParen) {
                            collect_param_indices(tokens, from_template, j, &mut bindings);
                            break;
                        }
                        j += 1;
                    }
                }
                // Find the `{` that opens the arms (after the scrutinee) and
                // collect each arm pattern's bindings.
                TokenKind::Match => {
                    let mut j = i + 1;
                    while j < tokens.len() {
                        if is_t(j) && matches!(tokens[j].kind, TokenKind::LBrace) {
                            collect_match_pattern_indices(tokens, from_template, j, &mut bindings);
                            break;
                        }
                        j += 1;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    bindings
}

/// Record the parameter names of a function or closure whose parameter list
/// opens at `open` (a `(`). A parameter is a template identifier immediately
/// followed by a `:` at the top level of the list (`fun(a: Int, b: String)`); a
/// spliced metavariable parameter keeps its caller identity and is skipped.
fn collect_param_indices(
    tokens: &[Token],
    from_template: &[bool],
    open: usize,
    bindings: &mut HashMap<usize, (String, BindKind)>,
) {
    let is_t = |i: usize| from_template.get(i).copied().unwrap_or(false);
    let mut depth = 0i32;
    let mut k = open;
    while k < tokens.len() {
        match &tokens[k].kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            TokenKind::Identifier(s) if depth == 1 && is_t(k) => {
                if is_t(k + 1)
                    && matches!(tokens.get(k + 1).map(|t| &t.kind), Some(TokenKind::Colon))
                {
                    bindings.insert(k, (s.clone(), BindKind::Param));
                }
            }
            _ => {}
        }
        k += 1;
    }
}

/// Record the pattern bindings of the match arms whose brace opens at `open`. A
/// pattern binding is a lowercase template identifier (Raven names constructors
/// and types in PascalCase) not in constructor position (not followed by `(` or
/// `{`). Bracket-depth tracking keeps an arm-separating comma and the `->` that
/// ends a pattern distinct from a comma or arrow nested in a pattern or body.
fn collect_match_pattern_indices(
    tokens: &[Token],
    from_template: &[bool],
    open: usize,
    bindings: &mut HashMap<usize, (String, BindKind)>,
) {
    let is_t = |i: usize| from_template.get(i).copied().unwrap_or(false);
    let mut depth = 0i32;
    let mut in_pattern = true;
    let mut k = open;
    while k < tokens.len() {
        match &tokens[k].kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            TokenKind::Arrow if depth == 1 => in_pattern = false,
            TokenKind::Comma if depth == 1 => in_pattern = true,
            TokenKind::Identifier(s) if in_pattern && is_t(k) && starts_lowercase(s) => {
                // Skip constructor heads (followed by `(` or `{`) and record
                // pattern field labels (followed by `:`, as in `Point { x: a }`):
                // the label is not the binding; the value pattern after it is.
                let non_binding = matches!(
                    tokens.get(k + 1).map(|t| &t.kind),
                    Some(TokenKind::LParen | TokenKind::LBrace | TokenKind::Colon)
                );
                if !non_binding {
                    bindings.insert(k, (s.clone(), BindKind::Pattern));
                }
            }
            _ => {}
        }
        k += 1;
    }
}

/// Whether `s` begins with an ASCII lowercase letter, the spelling convention
/// for a value binding (constructors and types are PascalCase).
fn starts_lowercase(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
}

/// Rename template-introduced bindings and their uses over the instantiated
/// token stream. A `Block` binding (`let`/`const`/`for`) adds a fresh gensym to
/// the current `{}` scope and only renames uses in that scope or a nested one,
/// so a nested local cannot rename a free use in an outer scope. A `Param`
/// binding is gensym'd and renamed across the whole expansion (it is not
/// brace-delimited). A `Pattern` binding is kept verbatim, since a shorthand
/// struct field is also the field name, but still recorded so its uses resolve.
/// A use with no binding is free: its span is recorded as a definition-site use
/// so the resolver resolves it against the macro's module scope. A captured
/// metavariable token (`!from_template[i]`) keeps its caller identity untouched.
fn scope_aware_rename(tokens: &mut [Token], from_template: &[bool], spans: &mut SpanGen) {
    let bindings = collect_binding_indices(tokens, from_template);
    let mut scopes: Vec<HashMap<String, String>> = vec![HashMap::new()];
    let mut flat: HashMap<String, String> = HashMap::new();
    for i in 0..tokens.len() {
        if !from_template.get(i).copied().unwrap_or(false) {
            continue;
        }
        match &tokens[i].kind {
            TokenKind::LBrace => scopes.push(HashMap::new()),
            TokenKind::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
            }
            TokenKind::Identifier(s) => {
                if let Some((name, kind)) = bindings.get(&i).cloned() {
                    let fresh = match kind {
                        BindKind::Block => {
                            let fresh = spans.gensym(&name);
                            scopes
                                .last_mut()
                                .expect("a scope is always present")
                                .insert(name, fresh.clone());
                            fresh
                        }
                        // One fresh name per spelling, shared by every use.
                        BindKind::Param => flat
                            .entry(name.clone())
                            .or_insert_with(|| spans.gensym(&name))
                            .clone(),
                        // Kept verbatim so a struct field name still matches; the
                        // identity entry lets its uses resolve instead of free.
                        BindKind::Pattern => flat
                            .entry(name.clone())
                            .or_insert_with(|| name.clone())
                            .clone(),
                    };
                    tokens[i].kind = TokenKind::Identifier(fresh);
                } else {
                    // A use: an in-scope block binding wins over a flat binding.
                    let resolved = scopes
                        .iter()
                        .rev()
                        .find_map(|scope| scope.get(s))
                        .or_else(|| flat.get(s))
                        .cloned();
                    match resolved {
                        Some(fresh) => tokens[i].kind = TokenKind::Identifier(fresh),
                        None => {
                            let span = &tokens[i].span;
                            spans.def_sites.insert((span.file.clone(), span.start));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Emit the instantiated tokens of `template` into `out`. Repetition groups are
/// expanded once per captured repetition, splicing the separator between
/// copies; sequence metavariables are indexed by the current repetition.
fn instantiate_into(
    template: &[TemplateItem],
    binds: &HashMap<String, Capture>,
    call_span: &Span,
    spans: &mut SpanGen,
    out: &mut Vec<Token>,
    from_template: &mut Vec<bool>,
) {
    for item in template {
        match item {
            TemplateItem::Meta(name) => {
                if let Some(cap) = binds.get(name) {
                    let toks: &[Token] = match cap {
                        Capture::Single(toks) => toks.as_slice(),
                        // A sequence metavariable used outside a repetition
                        // splices nothing; correct usage is under a template
                        // repetition, where the per-rep view is used.
                        Capture::Seq(_) => &[],
                    };
                    // Captured tokens keep their original spelling (no hygiene
                    // rename): they refer to caller bindings.
                    for t in toks {
                        out.push(Token::new(t.kind.clone(), spans.fresh(call_span)));
                        from_template.push(false);
                    }
                }
            }
            TemplateItem::Token(t) => {
                out.push(Token::new(t.kind.clone(), spans.fresh(call_span)));
                from_template.push(true);
            }
            TemplateItem::Rep { sub, sep } => {
                let count = rep_count(sub, binds);
                for r in 0..count {
                    if r > 0 {
                        if let Some(septok) = sep {
                            out.push(Token::new(septok.kind.clone(), spans.fresh(call_span)));
                            from_template.push(true);
                        }
                    }
                    let view = rep_view(binds, r);
                    instantiate_into(sub, &view, call_span, spans, out, from_template);
                }
            }
        }
    }
}

/// Number of repetitions for a template repetition: the length of the first
/// sequence-captured metavariable referenced in `sub`.
fn rep_count(sub: &[TemplateItem], binds: &HashMap<String, Capture>) -> usize {
    for name in template_meta_names(sub) {
        if let Some(Capture::Seq(seq)) = binds.get(&name) {
            return seq.len();
        }
    }
    0
}

/// Build a per-repetition binding view at index `r`: each sequence capture is
/// projected to its `r`th element (itself a capture, which may be a nested
/// sequence); single captures pass through unchanged.
fn rep_view(binds: &HashMap<String, Capture>, r: usize) -> HashMap<String, Capture> {
    let mut view = HashMap::new();
    for (k, v) in binds {
        match v {
            Capture::Seq(seq) => {
                if let Some(cap) = seq.get(r) {
                    view.insert(k.clone(), cap.clone());
                }
            }
            Capture::Single(_) => {
                view.insert(k.clone(), v.clone());
            }
        }
    }
    view
}

/// Names of metavariables referenced anywhere in a template slice.
fn template_meta_names(items: &[TemplateItem]) -> Vec<String> {
    let mut out = Vec::new();
    for it in items {
        match it {
            TemplateItem::Meta(n) => out.push(n.clone()),
            TemplateItem::Rep { sub, .. } => out.extend(template_meta_names(sub)),
            TemplateItem::Token(_) => {}
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
