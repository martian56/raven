use super::*;
use crate::lexer::Lexer;

fn lex(src: &str) -> Vec<Token> {
    Lexer::new(src, "test.rv").tokenize().expect("lex ok")
}

/// Expand `src` and render the resulting tokens (without `Eof`/`Newline`) as
/// a compact, comparable string for assertions.
fn expand_render(src: &str) -> String {
    let toks = expand_tokens(&lex(src)).expect("expand ok");
    render(&toks)
}

fn render(toks: &[Token]) -> String {
    let mut out = String::new();
    for t in toks {
        match &t.kind {
            TokenKind::Eof | TokenKind::Newline => continue,
            other => {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(&piece(other));
            }
        }
    }
    out
}

fn piece(k: &TokenKind) -> String {
    match k {
        TokenKind::Identifier(s) => s.clone(),
        TokenKind::IntLit(n) => n.to_string(),
        TokenKind::Plus => "+".into(),
        TokenKind::Star => "*".into(),
        TokenKind::Gt => ">".into(),
        TokenKind::LParen => "(".into(),
        TokenKind::RParen => ")".into(),
        TokenKind::LBrace => "{".into(),
        TokenKind::RBrace => "}".into(),
        TokenKind::Comma => ",".into(),
        TokenKind::If => "if".into(),
        TokenKind::Else => "else".into(),
        TokenKind::Let => "let".into(),
        TokenKind::Eq => "=".into(),
        TokenKind::Fun => "fun".into(),
        other => crate::parser::describe_token(other),
    }
}

#[test]
fn no_macros_is_a_noop() {
    let src = "fun main() {\n    let x = foo(1) + bar(2)\n}\n";
    let original = lex(src);
    let expanded = expand_tokens(&original).expect("expand ok");
    assert_eq!(
        original, expanded,
        "non-macro token stream must be unchanged"
    );
}

#[test]
fn single_expr_metavariable_expands_with_parens() {
    let src = "macro twice { ($x:expr) => { ($x) + ($x) } }\nlet y = twice!(n + 1)\n";
    // The captured `n + 1` is spliced into both `($x)` slots.
    assert_eq!(expand_render(src), "let y = ( n + 1 ) + ( n + 1 )");
}

#[test]
fn two_metavariables_match_by_comma_delimiter() {
    let src = "macro max2 { ($a:expr, $b:expr) => { if ($a) > ($b) { ($a) } else { ($b) } } }\n\
               let m = max2!(p * 2, q)\n";
    assert_eq!(
        expand_render(src),
        "let m = if ( p * 2 ) > ( q ) { ( p * 2 ) } else { ( q ) }"
    );
}

#[test]
fn ident_fragment_captures_one_identifier() {
    let src = "macro id { ($x:ident) => { $x } }\nlet z = id!(value)\n";
    assert_eq!(expand_render(src), "let z = value");
}

#[test]
fn nested_macro_calls_expand_to_fixpoint() {
    let src = "macro twice { ($x:expr) => { ($x) + ($x) } }\nlet y = twice!(twice!(k))\n";
    assert_eq!(
        expand_render(src),
        "let y = ( ( k ) + ( k ) ) + ( ( k ) + ( k ) )"
    );
}

#[test]
fn unknown_macro_is_an_error() {
    // A defined macro is present so the pass runs, but the call names another.
    let src = "macro twice { ($x:expr) => { ($x) } }\nlet y = nope!(1)\n";
    let e = expand_tokens(&lex(src)).expect_err("unknown macro");
    let msg = format!("{}", e);
    assert!(msg.contains("unknown macro `nope!`"), "got: {}", msg);
}

#[test]
fn arity_mismatch_is_an_error() {
    let src = "macro max2 { ($a:expr, $b:expr) => { ($a) } }\nlet m = max2!(1)\n";
    let e = expand_tokens(&lex(src)).expect_err("arity mismatch");
    let msg = format!("{}", e);
    assert!(msg.contains("no rule of macro `max2!`"), "got: {}", msg);
}

#[test]
fn expansion_limit_guards_recursion() {
    let src = "macro loopy { ($x:expr) => { loopy!($x) } }\nlet y = loopy!(1)\n";
    let e = expand_tokens(&lex(src)).expect_err("should hit the limit");
    let msg = format!("{}", e);
    assert!(msg.contains("expansion exceeded"), "got: {}", msg);
}

#[test]
fn first_matching_rule_wins() {
    let src = "macro pick { ($a:expr, $b:expr) => { two } ($a:expr) => { one } }\n\
               let r = pick!(x)\n";
    assert_eq!(expand_render(src), "let r = one");
}

#[test]
fn duplicate_macro_is_an_error() {
    let src = "macro dup { ($x:expr) => { ($x) } }\nmacro dup { ($x:expr) => { ($x) } }\n";
    let e = expand_tokens(&lex(src)).expect_err("duplicate def");
    assert!(format!("{}", e).contains("already defined"));
}
