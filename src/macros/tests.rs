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
        TokenKind::LBracket => "[".into(),
        TokenKind::RBracket => "]".into(),
        TokenKind::Colon => ":".into(),
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
fn ty_fragment_captures_a_balanced_type() {
    let src = "macro sizer { ($t:ty) => { size_of($t) } }\nlet n = sizer!(List<Int>)\n";
    assert_eq!(expand_render(src), "let n = size_of ( List `<` Int > )");
}

#[test]
fn literal_fragment_captures_one_literal() {
    let src = "macro dbl { ($x:literal) => { ($x) + ($x) } }\nlet y = dbl!(21)\n";
    assert_eq!(expand_render(src), "let y = ( 21 ) + ( 21 )");
}

#[test]
fn literal_fragment_rejects_a_non_literal() {
    let src = "macro dbl { ($x:literal) => { ($x) } }\nlet y = dbl!(n)\n";
    let e = expand_tokens(&lex(src)).expect_err("identifier is not a literal");
    let msg = format!("{}", e);
    assert!(msg.contains("no rule of macro `dbl!`"), "got: {}", msg);
}

#[test]
fn block_fragment_captures_a_brace_group() {
    let src = "macro run { ($b:block) => { $b } }\nrun!({ let a = 1 })\n";
    assert_eq!(expand_render(src), "{ let a = 1 }");
}

#[test]
fn pat_fragment_captures_a_pattern() {
    let src = "macro is { ($p:pat, $e:expr) => { match $e { $p -> true, _ -> false } } }\n\
               let b = is!(Some(n), x)\n";
    assert_eq!(
        expand_render(src),
        "let b = `match` x { Some ( n ) `->` `true` , _ `->` `false` }"
    );
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
fn macro_table_expands_a_snippet() {
    // A file's table (collected from the definitions) expands a snippet that
    // was lexed on its own, the path used for `"${...}"` interpolation.
    let file = lex("macro twice { ($x:expr) => { ($x) + ($x) } }\n");
    let table = collect_macro_table(&file).expect("table ok");
    assert!(!table.is_empty());
    let snippet = lex("twice!(n + 1)");
    let out = expand_with_table(&snippet, &table).expect("expand ok");
    assert_eq!(render(&out), "( n + 1 ) + ( n + 1 )");
}

#[test]
fn macro_table_is_empty_without_definitions() {
    let table = collect_macro_table(&lex("fun main() {}\n")).expect("table ok");
    assert!(table.is_empty());
    // An empty table leaves a snippet untouched, even one that looks like a
    // call (no definition means nothing to expand).
    let snippet = lex("foo!(1)");
    let out = expand_with_table(&snippet, &table).expect("noop");
    assert_eq!(out, snippet);
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

#[test]
fn star_repetition_binds_sequence_and_repeats_template() {
    let src = "macro sum_all { ($($x:expr),*) => { (0 $(+ ($x))*) } }\n\
               let s = sum_all!(1, 2, 3)\n";
    assert_eq!(expand_render(src), "let s = ( 0 + ( 1 ) + ( 2 ) + ( 3 ) )");
}

#[test]
fn star_repetition_accepts_zero_reps() {
    let src = "macro sum_all { ($($x:expr),*) => { (0 $(+ ($x))*) } }\nlet s = sum_all!()\n";
    assert_eq!(expand_render(src), "let s = ( 0 )");
}

#[test]
fn star_repetition_accepts_one_rep() {
    let src = "macro sum_all { ($($x:expr),*) => { (0 $(+ ($x))*) } }\nlet s = sum_all!(10)\n";
    assert_eq!(expand_render(src), "let s = ( 0 + ( 10 ) )");
}

#[test]
fn plus_repetition_requires_at_least_one() {
    let src = "macro sum_some { ($($x:expr),+) => { (0 $(+ ($x))+) } }\nlet s = sum_some!()\n";
    let e = expand_tokens(&lex(src)).expect_err("plus needs one");
    assert!(
        format!("{}", e).contains("no rule of macro `sum_some!`"),
        "got: {}",
        e
    );
}

#[test]
fn plus_repetition_matches_multiple() {
    let src = "macro sum_some { ($($x:expr),+) => { (0 $(+ ($x))+) } }\nlet s = sum_some!(4, 5)\n";
    assert_eq!(expand_render(src), "let s = ( 0 + ( 4 ) + ( 5 ) )");
}

#[test]
fn repetition_with_multiple_metavariables_per_rep() {
    let src = "macro pairs { ($($k:ident : $v:expr),*) => { [$(($k, $v)),*] } }\n\
               let p = pairs!(a : 1, b : 2)\n";
    assert_eq!(expand_render(src), "let p = [ ( a , 1 ) , ( b , 2 ) ]");
}

#[test]
fn hygiene_renames_template_binding_not_captured_name() {
    // The template introduces `tmp`. The captured `$x` is also `tmp`. After
    // expansion the template's `tmp` is renamed (carries a `$`), while the
    // spliced capture keeps the original `tmp`.
    let src = "macro doubled { ($x:expr) => { { let tmp = ($x); tmp + tmp } } }\n\
               let r = doubled!(tmp)\n";
    let out = expand_render(src);
    // The capture (`($x)` -> `( tmp )`) keeps the bare name.
    assert!(out.contains("( tmp )"), "capture not preserved: {}", out);
    // The template binding and its uses are renamed away from `tmp`.
    assert!(out.contains("let tmp$"), "binding not renamed: {}", out);
    // No bare `let tmp ` remains (the introduced binding must be gensym'd).
    assert!(!out.contains("let tmp "), "binding leaked: {}", out);
}

#[test]
fn hygiene_renames_are_consistent_within_one_expansion() {
    let src = "macro doubled { ($x:expr) => { { let tmp = ($x); tmp + tmp } } }\n\
               let r = doubled!(7)\n";
    let out = expand_render(src);
    // Collect the renamed `tmp$N` spellings; the binding and both uses must
    // share one fresh name.
    let names: Vec<&str> = out
        .split_whitespace()
        .filter(|w| w.starts_with("tmp$"))
        .collect();
    assert_eq!(names.len(), 3, "expected one binding and two uses: {}", out);
    assert!(
        names.iter().all(|n| *n == names[0]),
        "inconsistent rename: {}",
        out
    );
}

#[test]
fn missing_repetition_marker_is_an_error() {
    let src = "macro bad { ($($x:expr),) => { ($x) } }\nlet s = bad!(1)\n";
    let e = expand_tokens(&lex(src)).expect_err("missing marker");
    assert!(
        format!("{}", e).contains("must end with `*` or `+`"),
        "got: {}",
        e
    );
}
