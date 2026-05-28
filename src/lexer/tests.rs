use super::*;
use crate::error::{LexError, RavenError};

fn lex(src: &str) -> Vec<Token> {
    Lexer::new(src, "test.rv").tokenize().expect("lex ok")
}

fn lex_err(src: &str) -> RavenError {
    Lexer::new(src, "test.rv")
        .tokenize()
        .expect_err("expected lex error")
}

fn kinds(tokens: &[Token]) -> Vec<TokenKind> {
    tokens.iter().map(|t| t.kind.clone()).collect()
}

#[test]
fn empty_source_yields_eof_only() {
    let toks = lex("");
    assert_eq!(toks.len(), 1);
    assert_eq!(toks[0].kind, TokenKind::Eof);
}

#[test]
fn all_keywords_are_recognized() {
    let src = "let const fun return if else while for loop in break continue match struct trait impl enum import as extern defer true false self Self";
    let toks = lex(src);
    let kinds = kinds(&toks);
    assert_eq!(
        kinds,
        vec![
            TokenKind::Let,
            TokenKind::Const,
            TokenKind::Fun,
            TokenKind::Return,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::While,
            TokenKind::For,
            TokenKind::Loop,
            TokenKind::In,
            TokenKind::Break,
            TokenKind::Continue,
            TokenKind::Match,
            TokenKind::Struct,
            TokenKind::Trait,
            TokenKind::Impl,
            TokenKind::Enum,
            TokenKind::Import,
            TokenKind::As,
            TokenKind::Extern,
            TokenKind::Defer,
            TokenKind::True,
            TokenKind::False,
            TokenKind::SelfLower,
            TokenKind::SelfUpper,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn identifiers_vs_keywords() {
    let toks = lex("letx let_ _let Int String foo123");
    assert_eq!(
        kinds(&toks),
        vec![
            TokenKind::Identifier("letx".into()),
            TokenKind::Identifier("let_".into()),
            TokenKind::Identifier("_let".into()),
            TokenKind::Identifier("Int".into()),
            TokenKind::Identifier("String".into()),
            TokenKind::Identifier("foo123".into()),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn integer_literals_in_all_four_bases_with_underscores() {
    let toks = lex("42 1_000_000 0xFF_FF 0b1010_0101 0o755");
    assert_eq!(
        kinds(&toks),
        vec![
            TokenKind::IntLit(42),
            TokenKind::IntLit(1_000_000),
            TokenKind::IntLit(0xFFFF),
            TokenKind::IntLit(0b1010_0101),
            TokenKind::IntLit(0o755),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn float_literals_with_and_without_exponent() {
    let toks = lex("3.25 1.0e10 6.022e-23 1E+3 1e10");
    let expected = [3.25_f64, 1.0e10, 6.022e-23, 1.0e3, 1.0e10];
    let nums: Vec<f64> = toks
        .iter()
        .filter_map(|t| match t.kind {
            TokenKind::FloatLit(v) => Some(v),
            _ => None,
        })
        .collect();
    assert_eq!(nums.len(), expected.len());
    for (a, b) in nums.iter().zip(expected.iter()) {
        assert!((a - b).abs() < 1e-9 || (a - b).abs() / b.abs() < 1e-12);
    }
}

#[test]
fn string_with_escape_sequences_is_cooked() {
    let toks = lex(r#""hello\n\t\"world\"\\""#);
    match &toks[0].kind {
        TokenKind::StringLit(s) => assert_eq!(s, "hello\n\t\"world\"\\"),
        other => panic!("expected StringLit, got {:?}", other),
    }
}

#[test]
fn string_preserves_interpolation_verbatim() {
    let src = r#""hello, ${name}!""#;
    let toks = lex(src);
    match &toks[0].kind {
        TokenKind::StringLit(s) => assert_eq!(s, "hello, ${name}!"),
        other => panic!("expected StringLit, got {:?}", other),
    }
}

#[test]
fn string_supports_hex_and_unicode_escapes() {
    let toks = lex(r#""\x41\u{1F600}""#);
    match &toks[0].kind {
        TokenKind::StringLit(s) => {
            assert_eq!(s.chars().next(), Some('A'));
            assert_eq!(s.chars().nth(1), Some('\u{1F600}'));
        }
        other => panic!("expected StringLit, got {:?}", other),
    }
}

#[test]
fn block_string_preserves_whitespace_and_newlines() {
    let src = "\"\"\"\n    line1\n    line2\n\"\"\"";
    let toks = lex(src);
    match &toks[0].kind {
        TokenKind::BlockStringLit(s) => assert_eq!(s, "\n    line1\n    line2\n"),
        other => panic!("expected BlockStringLit, got {:?}", other),
    }
}

#[test]
fn char_literal_basic_and_escaped() {
    let toks = lex(r#"'a' '\n' '\u{1F600}'"#);
    let chars: Vec<char> = toks
        .iter()
        .filter_map(|t| match t.kind {
            TokenKind::CharLit(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(chars, vec!['a', '\n', '\u{1F600}']);
}

#[test]
fn c_string_literal_is_distinct_kind() {
    let toks = lex(r#"c"hello""#);
    match &toks[0].kind {
        TokenKind::CStringLit(s) => assert_eq!(s, "hello"),
        other => panic!("expected CStringLit, got {:?}", other),
    }
}

#[test]
fn operators_use_longest_match() {
    let toks = lex("+ += - -= * *= / /= % %= = == => != < <= << <<= > >= >> >>= && & &= || | |= ^ ^= ~ ! .. ..= . :: : ? -> @");
    let expected = vec![
        TokenKind::Plus,
        TokenKind::PlusEq,
        TokenKind::Minus,
        TokenKind::MinusEq,
        TokenKind::Star,
        TokenKind::StarEq,
        TokenKind::Slash,
        TokenKind::SlashEq,
        TokenKind::Percent,
        TokenKind::PercentEq,
        TokenKind::Eq,
        TokenKind::EqEq,
        TokenKind::FatArrow,
        TokenKind::NotEq,
        TokenKind::Lt,
        TokenKind::LtEq,
        TokenKind::Shl,
        TokenKind::ShlEq,
        TokenKind::Gt,
        TokenKind::GtEq,
        TokenKind::Shr,
        TokenKind::ShrEq,
        TokenKind::AndAnd,
        TokenKind::Amp,
        TokenKind::AmpEq,
        TokenKind::OrOr,
        TokenKind::Pipe,
        TokenKind::PipeEq,
        TokenKind::Caret,
        TokenKind::CaretEq,
        TokenKind::Tilde,
        TokenKind::Bang,
        TokenKind::DotDot,
        TokenKind::DotDotEq,
        TokenKind::Dot,
        TokenKind::ColonColon,
        TokenKind::Colon,
        TokenKind::Question,
        TokenKind::Arrow,
        TokenKind::At,
        TokenKind::Eof,
    ];
    assert_eq!(kinds(&toks), expected);
}

#[test]
fn dotdot_vs_dotdoteq_longest_match() {
    let toks = lex("0..10 0..=10");
    assert_eq!(
        kinds(&toks),
        vec![
            TokenKind::IntLit(0),
            TokenKind::DotDot,
            TokenKind::IntLit(10),
            TokenKind::IntLit(0),
            TokenKind::DotDotEq,
            TokenKind::IntLit(10),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn punctuation_tokens() {
    let toks = lex("( ) { } [ ] , ; : @");
    assert_eq!(
        kinds(&toks),
        vec![
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::LBrace,
            TokenKind::RBrace,
            TokenKind::LBracket,
            TokenKind::RBracket,
            TokenKind::Comma,
            TokenKind::Semi,
            TokenKind::Colon,
            TokenKind::At,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn line_comments_are_stripped() {
    let toks = lex("let x // trailing comment\n= 1");
    assert_eq!(
        kinds(&toks),
        vec![
            TokenKind::Let,
            TokenKind::Identifier("x".into()),
            TokenKind::Newline,
            TokenKind::Eq,
            TokenKind::IntLit(1),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn block_comments_are_stripped() {
    let toks = lex("a /* multi\nline */ b");
    assert_eq!(
        kinds(&toks),
        vec![
            TokenKind::Identifier("a".into()),
            TokenKind::Identifier("b".into()),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn multiple_newlines_coalesce_into_one_newline_token() {
    let toks = lex("a\n\n\nb\r\n\r\nc");
    assert_eq!(
        kinds(&toks),
        vec![
            TokenKind::Identifier("a".into()),
            TokenKind::Newline,
            TokenKind::Identifier("b".into()),
            TokenKind::Newline,
            TokenKind::Identifier("c".into()),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn spans_track_line_and_column() {
    let toks = lex("foo\n  bar");
    assert_eq!(toks[0].span.line, 1);
    assert_eq!(toks[0].span.col, 1);
    // toks[1] = Newline
    assert_eq!(toks[2].span.line, 2);
    assert_eq!(toks[2].span.col, 3);
}

#[test]
fn eof_span_is_zero_width_at_end() {
    let src = "abc";
    let toks = lex(src);
    let eof = toks.last().unwrap();
    assert_eq!(eof.kind, TokenKind::Eof);
    assert!(eof.span.is_empty());
    assert_eq!(eof.span.start, src.len());
}

// ----- error tests -----

#[test]
fn unexpected_char_reports_span() {
    let err = lex_err("let x = #");
    match err {
        RavenError::Lex(LexError::UnexpectedChar('#'), span, _) => {
            assert_eq!(span.line, 1);
            assert_eq!(span.col, 9);
        }
        other => panic!("expected UnexpectedChar('#'), got {:?}", other),
    }
}

#[test]
fn unterminated_string_is_error() {
    let err = lex_err("\"hello");
    assert!(matches!(
        err,
        RavenError::Lex(LexError::UnterminatedString, _, _)
    ));
}

#[test]
fn newline_inside_string_is_unterminated() {
    let err = lex_err("\"hello\nworld\"");
    assert!(matches!(
        err,
        RavenError::Lex(LexError::UnterminatedString, _, _)
    ));
}

#[test]
fn unterminated_block_string_is_error() {
    let err = lex_err("\"\"\"unfinished");
    assert!(matches!(
        err,
        RavenError::Lex(LexError::UnterminatedBlockString, _, _)
    ));
}

#[test]
fn unterminated_block_comment_is_error() {
    let err = lex_err("/* never closed");
    assert!(matches!(
        err,
        RavenError::Lex(LexError::UnterminatedBlockComment, _, _)
    ));
}

#[test]
fn invalid_escape_is_error() {
    let err = lex_err(r#""bad \q escape""#);
    assert!(matches!(
        err,
        RavenError::Lex(LexError::InvalidEscape('q'), _, _)
    ));
}

#[test]
fn invalid_unicode_escape_short_hex() {
    let err = lex_err(r#""\xZZ""#);
    assert!(matches!(
        err,
        RavenError::Lex(LexError::InvalidUnicodeEscape, _, _)
    ));
}

#[test]
fn invalid_unicode_escape_unclosed_braces() {
    let err = lex_err(r#""\u{12""#);
    assert!(matches!(
        err,
        RavenError::Lex(LexError::InvalidUnicodeEscape, _, _)
    ));
}

#[test]
fn invalid_number_overflow_is_error() {
    let err = lex_err("99999999999999999999");
    assert!(matches!(
        err,
        RavenError::Lex(LexError::InvalidNumber(_), _, _)
    ));
}

#[test]
fn hex_literal_without_digits_is_error() {
    let err = lex_err("0x");
    assert!(matches!(
        err,
        RavenError::Lex(LexError::InvalidNumber(_), _, _)
    ));
}

#[test]
fn float_exponent_without_digits_is_error() {
    let err = lex_err("1e");
    assert!(matches!(
        err,
        RavenError::Lex(LexError::InvalidNumber(_), _, _)
    ));
}

#[test]
fn invalid_char_literal_empty_is_error() {
    let err = lex_err("''");
    assert!(matches!(
        err,
        RavenError::Lex(LexError::InvalidCharLit(_), _, _)
    ));
}

#[test]
fn invalid_char_literal_too_long_is_error() {
    let err = lex_err("'ab'");
    assert!(matches!(
        err,
        RavenError::Lex(LexError::InvalidCharLit(_), _, _)
    ));
}

#[test]
fn raven_error_display_includes_source_pointer() {
    let src = "let x = #\n";
    let err = lex_err(src);
    let rendered = err.display(src);
    assert!(rendered.contains("test.rv:1:9"));
    assert!(rendered.contains("let x = #"));
    assert!(rendered.contains('^'));
}

// ----- broader integration smoke test -----

#[test]
fn small_program_lexes_cleanly() {
    let src = r#"
fun add(a: Int, b: Int) -> Int = a + b

let x = 5
let s = "hello, ${name}"
"#;
    let toks = lex(src);
    // Just make sure we got a reasonable number of tokens and the structure
    // is plausible (begins with a Newline, ends with Eof).
    assert_eq!(toks.last().unwrap().kind, TokenKind::Eof);
    assert!(toks.iter().any(|t| matches!(t.kind, TokenKind::Fun)));
    assert!(toks
        .iter()
        .any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "add")));
    assert!(toks.iter().any(|t| matches!(t.kind, TokenKind::Arrow)));
    assert!(toks
        .iter()
        .any(|t| matches!(&t.kind, TokenKind::StringLit(s) if s == "hello, ${name}")));
}
