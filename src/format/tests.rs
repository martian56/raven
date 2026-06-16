use super::{format_source, format_source_with};
use crate::ast::pretty_file;

#[test]
fn indent_width_is_configurable() {
    let out = format_source_with("fun main() {\nlet x = 1\n}\n", 2).unwrap();
    assert!(
        out.contains("\n  let x = 1"),
        "expected a two-space indent, got: {out:?}"
    );
    let four = format_source_with("fun main() {\nlet x = 1\n}\n", 4).unwrap();
    assert!(
        four.contains("\n    let x = 1"),
        "expected a four-space indent"
    );
}
use crate::lexer::Lexer;
use crate::parser::parse;

fn ast_string(src: &str) -> String {
    let tokens = Lexer::new(src, "<t>").tokenize().expect("lex");
    let file = parse(&tokens).expect("parse");
    pretty_file(&file)
}

/// Format, then assert idempotency and semantic preservation.
fn fmt(src: &str) -> String {
    let once = format_source(src).expect("format once");
    let twice = format_source(&once).expect("format twice");
    assert_eq!(once, twice, "formatter is not idempotent");
    assert_eq!(
        ast_string(src),
        ast_string(&once),
        "formatting changed the AST"
    );
    once
}

#[test]
fn function_block_body() {
    let out = fmt("fun   main ( )  {\nlet x=1\nreturn x\n}");
    assert_eq!(out, "fun main() {\n    let x = 1\n    return x\n}\n");
}

#[test]
fn comment_inside_call_args_stays_in_place() {
    // A comment interior to a call must stay with its argument and a later
    // standalone comment must keep its place, not be relocated to end of file.
    // Regression for #426.
    let src = "fun main() {\n    let r = foo(\n        1, // first arg comment\n        2,\n    )\n    print(r)\n    // trailing standalone comment\n    bar()\n}\n";
    let out = fmt(src);
    assert!(
        out.contains("1, // first arg comment"),
        "interior call comment was lost or moved: {out:?}"
    );
    let comment_pos = out.find("// trailing standalone comment").unwrap();
    let bar_pos = out.find("bar()").unwrap();
    assert!(
        comment_pos < bar_pos,
        "standalone comment was relocated past its code: {out:?}"
    );
}

#[test]
fn comment_before_first_array_element_stays_inside_brackets() {
    // A comment between the opening `[` and the first element must keep the
    // array multi-line with the comment in place. Before #532 the multi-line
    // decision window started at the first element, so this comment fell
    // outside it: the array collapsed to one line and the comment was lost or
    // relocated.
    let src = "fun main() {\n    let xs = [\n        // the leading value\n        1,\n        2,\n    ]\n    print(xs)\n}\n";
    let out = fmt(src);
    let comment_pos = out
        .find("// the leading value")
        .expect("leading comment preserved");
    let open_pos = out.find('[').expect("array opens");
    let first_elem_pos = out.find('1').expect("first element present");
    assert!(
        open_pos < comment_pos && comment_pos < first_elem_pos,
        "comment should sit between `[` and the first element: {out:?}"
    );
}

#[test]
fn comment_after_call_open_paren_stays_with_first_arg() {
    // The same window fix applies to call arguments: a comment right after the
    // opening `(`, before the first argument, must keep the call multi-line
    // with the comment in place.
    let src = "fun main() {\n    let r = foo(\n        // the first argument\n        1,\n        2,\n    )\n    print(r)\n}\n";
    let out = fmt(src);
    let comment_pos = out
        .find("// the first argument")
        .expect("leading arg comment preserved");
    let call_pos = out.find("foo(").expect("call present");
    let first_arg_pos = out.find('1').expect("first argument present");
    assert!(
        call_pos < comment_pos && comment_pos < first_arg_pos,
        "comment should sit between `foo(` and the first argument: {out:?}"
    );
}

#[test]
fn doc_comment_stays_attached_to_derived_item() {
    // A comment directly above `@derive` documents the item, so the formatter
    // must not split them with a blank line (which `rvpm doc` would read as the
    // comment no longer documenting the item).
    let src = "// A point.\n@derive(Eq, Ord)\nstruct Point {\n    x: Int,\n}\n";
    let out = fmt(src);
    assert!(
        out.contains("// A point.\n@derive(Eq, Ord)"),
        "comment was split from @derive: {out:?}"
    );
}

#[test]
fn intentional_blank_above_derived_item_is_preserved() {
    // A real blank line between a comment and a derived item is kept.
    let src = "// not the doc\n\n@derive(Eq)\nenum E {\n    A,\n}\n";
    let out = fmt(src);
    assert!(
        out.contains("// not the doc\n\n@derive(Eq)"),
        "intentional blank line was dropped: {out:?}"
    );
}

#[test]
fn call_without_comments_stays_single_line() {
    let out = fmt("fun main() {\n    let r = foo(\n        1,\n        2,\n    )\n}\n");
    assert!(
        out.contains("foo(1, 2)"),
        "a comment-free call should collapse to one line: {out:?}"
    );
}

#[test]
fn function_expr_body() {
    let out = fmt("fun double(x:Int)->Int=x*2");
    assert_eq!(out, "fun double(x: Int) -> Int = x * 2\n");
}

#[test]
fn generics_and_bounds() {
    let out = fmt("fun id<T:Ord+Clone>(x:T)->T{return x}");
    assert_eq!(
        out,
        "fun id<T: Ord + Clone>(x: T) -> T {\n    return x\n}\n"
    );
}

#[test]
fn struct_decl() {
    let out = fmt("struct Point{x:Int,y:Int}");
    assert_eq!(out, "struct Point {\n    x: Int,\n    y: Int,\n}\n");
}

#[test]
fn enum_with_payloads() {
    let out = fmt("enum E{A,B(Int,String),C(x:Int)}");
    assert_eq!(
        out,
        "enum E {\n    A,\n    B(Int, String),\n    C(x: Int),\n}\n"
    );
}

#[test]
fn trait_and_impl() {
    let out = fmt("trait T{fun f(self)->Int}\nimpl T for Int{fun f(self)->Int{return 0}}");
    assert!(out.contains("trait T {"));
    assert!(out.contains("impl T for Int {"));
    assert!(out.contains("fun f(self) -> Int {"));
}

#[test]
fn match_with_guard() {
    let src = "fun f(x:Int)->Int{return match x{0->1,n if n>0->2,_->3}}";
    let out = fmt(src);
    assert!(out.contains("match x {"));
    assert!(out.contains("        0 -> 1,"));
    assert!(out.contains("        n if n > 0 -> 2,"));
    assert!(out.contains("        _ -> 3,"));
}

#[test]
fn if_expression() {
    let out = fmt("fun f()->Int{let y=if true{1}else{2}\nreturn y}");
    assert!(out.contains("let y = if true {"));
    assert!(out.contains("} else {"));
}

#[test]
fn for_loop_and_range() {
    let out = fmt("fun f(){for i in 0..10{print(i)}}");
    assert!(out.contains("for i in 0..10 {"));
}

#[test]
fn while_loop() {
    let out = fmt("fun f(){while x<10{x=x+1}}");
    assert!(out.contains("while x < 10 {"));
}

#[test]
fn defer_stmt() {
    let out = fmt("fun f(){defer cleanup()}");
    assert!(out.contains("    defer cleanup()"));
}

#[test]
fn macro_definition_and_call_are_formatted() {
    // A cramped macro definition and the function that uses it both
    // canonicalize: the rule gets spacing, the metavariable stays tight, and
    // the `name!(...)` call is laid out like any other expression.
    let src = "macro square{($x:expr)=>{($x)*($x)}}\nfun main(){\nlet n=square!(5)\nprint(n)\n}\n";
    let out = format_source(src).expect("formatting a macro file must not error");
    let expected = "macro square { ($x:expr) => { ($x) * ($x) } }\nfun main() {\n    let n = square!(5)\n    print(n)\n}\n";
    assert_eq!(out, expected);
}

#[test]
fn multi_rule_macro_splits_onto_lines() {
    let src = "macro pick{($a:expr,$b:expr)=>{two}($a:expr)=>{one}}\n";
    let out = format_source(src).expect("format ok");
    let expected = "macro pick {\n    ($a:expr, $b:expr) => { two }\n    ($a:expr) => { one }\n}\n";
    assert_eq!(out, expected);
}

#[test]
fn macro_formatting_is_idempotent() {
    let src = "macro twice{($x:expr)=>{($x)+($x)}}\nfun main(){print(twice!(3))}\n";
    let once = format_source(src).expect("format ok");
    let twice = format_source(&once).expect("format ok");
    assert_eq!(
        once, twice,
        "formatting an already-formatted macro file is a no-op"
    );
}

#[test]
fn macro_call_with_bracket_delimiter() {
    let out = fmt("fun f(){let a=make![1,2,3]\n a}");
    assert!(out.contains("make![1, 2, 3]"), "got: {out}");
}

#[test]
fn spawn_stmt_formats_as_call() {
    // `spawn` reads as a call: no space before the parenthesis, a single
    // paren layer, and stable under re-formatting (checked by `fmt`).
    let out = fmt("fun f(){spawn (fun()->Unit{work()})}");
    assert!(out.contains("spawn(fun() -> Unit {"), "got: {out}");
    assert!(!out.contains("spawn ("), "got: {out}");
}

#[test]
fn extern_block() {
    let out = fmt("extern \"C\"{fun foo(x:Int)->Int\nfun bar()}");
    assert!(out.contains("extern \"C\" {"));
    assert!(out.contains("    fun foo(x: Int) -> Int"));
    assert!(out.contains("    fun bar()"));
}

#[test]
fn imports() {
    let out = fmt("import std/io\nimport std/collections{Map,Set}\nimport \"./local\" as loc");
    assert!(out.contains("import std/io\n"));
    assert!(out.contains("import std/collections { Map, Set }"));
    assert!(out.contains("import \"./local\" as loc"));
}

#[test]
fn interpolation() {
    let out = fmt("fun f(n:Int)->String{return \"value=${n}\"}");
    assert!(out.contains("\"value=${n}\""));
}

#[test]
fn method_chain() {
    let out = fmt("fun f(xs:List<Int>)->Int{return xs.map(g).filter(h).len()}");
    assert!(out.contains("xs.map(g).filter(h).len()"));
}

#[test]
fn compound_assignment() {
    let out = fmt("fun f(){x+=1\ny*=2}");
    assert!(out.contains("    x += 1"));
    assert!(out.contains("    y *= 2"));
}

#[test]
fn lambda_forms() {
    let out = fmt("fun f(){let g={x->x+1}\nlet h=fun(a:Int)->Int=a}");
    assert!(out.contains("{ x -> x + 1 }"));
    assert!(out.contains("fun(a: Int) -> Int = a"));
}

#[test]
fn collapse_blank_lines() {
    let out = fmt("fun a(){}\n\n\n\nfun b(){}");
    assert_eq!(out, "fun a() {}\n\nfun b() {}\n");
}

#[test]
fn float_literal_keeps_point() {
    let out = fmt("const PI: Float = 3.0");
    assert_eq!(out, "const PI: Float = 3.0\n");
}

#[test]
fn leading_comment_preserved() {
    let out = fmt("// a header\nfun main(){}");
    assert_eq!(out, "// a header\nfun main() {}\n");
}

#[test]
fn trailing_comment_preserved() {
    let out = fmt("fun main(){let x=1 // count\n}");
    assert!(out.contains("let x = 1 // count"));
}

#[test]
fn comment_between_items() {
    let src = "fun a() {}\n\n// describe b\nfun b() {}\n";
    let out = fmt(src);
    assert!(out.contains("// describe b\nfun b() {}"));
}

#[test]
fn nested_block_indentation() {
    let src = "fun f(){if true{if false{return 1}}}";
    let out = fmt(src);
    assert!(out.contains("    if true {"));
    assert!(out.contains("        if false {"));
    assert!(out.contains("            return 1"));
}

#[test]
fn multiline_struct_literal_preserved() {
    let src = "fun f()->P{return P{\nx: 1,\ny: 2,\n}}";
    let out = fmt(src);
    assert!(out.contains("P {\n        x: 1,\n        y: 2,\n    }"));
}

#[test]
fn inline_struct_literal_stays_inline() {
    let src = "fun f()->P{return P{x: 1, y: 2}}";
    let out = fmt(src);
    assert!(out.contains("P { x: 1, y: 2 }"));
}

#[test]
fn empty_input() {
    assert_eq!(format_source("").unwrap(), "");
    assert_eq!(format_source("\n\n").unwrap(), "");
}

#[test]
fn dollar_brace_in_plain_string_escaped() {
    // A literal `${` (written escaped in source) must stay escaped so it
    // does not re-parse as interpolation.
    let out = fmt("fun f()->String{return \"\\${x}\"}");
    assert!(out.contains("\\${x}"));
}

#[test]
fn set_literal_round_trips() {
    let out = fmt("fun f(){let s={1,2,  3}}");
    assert_eq!(out, "fun f() {\n    let s = {1, 2, 3}\n}\n");
}

#[test]
fn single_element_set_keeps_one_element() {
    let out = fmt("fun f(){let s={1,}}");
    assert_eq!(out, "fun f() {\n    let s = {1,}\n}\n");
}

#[test]
fn map_literal_round_trips() {
    let out = fmt("fun f(){let m=[\"a\":1,\"b\":2]}");
    assert_eq!(out, "fun f() {\n    let m = [\"a\": 1, \"b\": 2]\n}\n");
}

#[test]
fn empty_map_literal_round_trips() {
    let out = fmt("fun f(){let m=[:]}");
    assert_eq!(out, "fun f() {\n    let m = [:]\n}\n");
}

#[test]
fn empty_list_stays_a_list() {
    let out = fmt("fun f(){let e=[]}");
    assert_eq!(out, "fun f() {\n    let e = []\n}\n");
}

#[test]
fn comment_inside_struct_literal_is_kept() {
    // A comment interior to a struct literal must survive formatting. Before
    // #579 the struct-literal printer ignored the comment cursor and the
    // comment was relocated past the literal. Regression for #579.
    let src = "fun f() {\n    let p = Point {\n        x: 1, // the x\n        // the y\n        y: 2,\n    }\n}\n";
    let out = fmt(src);
    assert!(
        out.contains("x: 1, // the x"),
        "trailing field comment lost: {out:?}"
    );
    assert!(
        out.contains("// the y"),
        "own-line field comment lost: {out:?}"
    );
}

#[test]
fn comment_inside_map_literal_is_kept() {
    // Regression for #579: a comment inside a map literal must be kept and the
    // literal laid out multi-line.
    let src = "fun f() {\n    let m = [\n        \"a\": 1, // first\n        \"b\": 2,\n    ]\n}\n";
    let out = fmt(src);
    assert!(
        out.contains("\"a\": 1, // first"),
        "map entry comment lost: {out:?}"
    );
}

#[test]
fn comment_inside_match_is_kept() {
    // Regression for #579: comments between and after match arms must survive.
    let src = "fun f(x: Int) -> Int {\n    match x {\n        // the zero case\n        0 -> 1, // one\n        _ -> 2,\n    }\n}\n";
    let out = fmt(src);
    assert!(
        out.contains("// the zero case"),
        "own-line arm comment lost: {out:?}"
    );
    assert!(
        out.contains("0 -> 1, // one"),
        "trailing arm comment lost: {out:?}"
    );
}

#[test]
fn interpolation_with_nested_string_does_not_break_comment_scan() {
    // A string interpolation may itself contain a string literal. The comment
    // scanner must skip the whole interpolation so the inner `"` does not end
    // the outer literal and mis-scan a following `//`. Regression for #580.
    let src = "fun f(name: String) {\n    let s = \"hi ${name.concat(\"!\")}\" // a greeting\n    print(s)\n}\n";
    let out = fmt(src);
    assert!(
        out.contains("// a greeting"),
        "trailing comment after an interpolated string was lost: {out:?}"
    );
}
