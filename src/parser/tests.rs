//! Inline unit tests for the parser. Each test takes a source string,
//! runs the lexer to produce tokens, and parses them, asserting on the
//! shape of the resulting AST or on the expected `RavenError`.
//!
//! These tests are intentionally focused on parser behavior; they do
//! not exercise the lexer beyond what is needed to set up a test.

use crate::ast::{
    BinaryOp, DeclKind, ExprKind, FunctionBody, ImportSource, LiteralPattern, PatternKind,
    StmtKind, TypeKind, UnaryOp, VariantPayload,
};
use crate::error::{ParseError, RavenError};
use crate::lexer::Lexer;

use super::{parse, parse_with_macros_all};

fn tokens(src: &str) -> Vec<crate::lexer::Token> {
    Lexer::new(src, "test.rv").tokenize().expect("lex ok")
}

/// Parse with item-level recovery and return every collected error (empty
/// when the source parses cleanly).
fn parse_all_errors(src: &str) -> Vec<RavenError> {
    let toks = tokens(src);
    match parse_with_macros_all(&toks, crate::macros::MacroTable::default()) {
        Ok(_) => Vec::new(),
        Err(es) => es,
    }
}

fn parse_ok(src: &str) -> crate::ast::File {
    let toks = tokens(src);
    parse(&toks).unwrap_or_else(|e| panic!("expected parse ok, got: {}", e))
}

fn parse_err(src: &str) -> RavenError {
    let toks = tokens(src);
    parse(&toks).expect_err("expected parse error")
}

// ----- expressions -----

#[test]
fn empty_file_has_no_items() {
    let f = parse_ok("");
    assert_eq!(f.items.len(), 0);
}

#[test]
fn top_level_bare_expression_is_an_error() {
    let err = parse_err("42\n");
    assert!(matches!(err, RavenError::Parse(_, _, _)), "got: {}", err);
}

#[test]
fn parses_let_decl_with_initializer() {
    let f = parse_ok("let x = 42\n");
    assert_eq!(f.items.len(), 1);
    let DeclKind::Let(let_decl) = &f.items[0].kind else {
        panic!("expected let decl");
    };
    assert_eq!(let_decl.name, "x");
    assert!(let_decl.ty.is_none());
    assert!(matches!(
        let_decl.init.as_ref().unwrap().kind,
        ExprKind::Int(42)
    ));
}

#[test]
fn parses_let_with_type_annotation() {
    let f = parse_ok("let n: Int = 7\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    assert!(d.ty.is_some());
    assert!(matches!(&d.ty.as_ref().unwrap().kind, TypeKind::Path(_)));
}

#[test]
fn parses_binary_arithmetic() {
    let f = parse_ok("let x = 1 + 2 * 3\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::Binary { op, lhs, rhs } = &d.init.as_ref().unwrap().kind else {
        panic!("expected Binary, got {:?}", d.init);
    };
    assert_eq!(*op, BinaryOp::Add);
    assert!(matches!(lhs.kind, ExprKind::Int(1)));
    let ExprKind::Binary { op: op2, .. } = &rhs.kind else {
        panic!("expected nested Binary");
    };
    assert_eq!(*op2, BinaryOp::Mul);
}

#[test]
fn parses_unary_negation_and_not() {
    let f = parse_ok("let x = -!a\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::Unary { op, operand } = &d.init.as_ref().unwrap().kind else {
        panic!();
    };
    assert_eq!(*op, UnaryOp::Neg);
    let ExprKind::Unary { op: op2, .. } = &operand.kind else {
        panic!();
    };
    assert_eq!(*op2, UnaryOp::Not);
}

#[test]
fn chained_comparison_is_rejected() {
    let err = parse_err("let x = a < b < c\n");
    assert!(
        matches!(err, RavenError::Parse(ParseError::ChainedComparison, _, _)),
        "got: {}",
        err
    );
}

#[test]
fn parses_call_and_method_chain() {
    let f = parse_ok("let x = foo(1, 2).bar().baz\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    // Outer: Field with name baz
    let ExprKind::Field { name, receiver } = &d.init.as_ref().unwrap().kind else {
        panic!("expected Field");
    };
    assert_eq!(name, "baz");
    let ExprKind::MethodCall {
        name: mname, args, ..
    } = &receiver.kind
    else {
        panic!("expected MethodCall");
    };
    assert_eq!(mname, "bar");
    assert_eq!(args.len(), 0);
}

#[test]
fn parses_index_and_try() {
    let f = parse_ok("let x = arr[0]?\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::Try(inner) = &d.init.as_ref().unwrap().kind else {
        panic!()
    };
    let ExprKind::Index { .. } = inner.kind else {
        panic!()
    };
}

#[test]
fn parses_range_expr() {
    let f = parse_ok("let r = 0..10\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::Range { inclusive, .. } = &d.init.as_ref().unwrap().kind else {
        panic!()
    };
    assert!(!*inclusive);
}

#[test]
fn parses_array_literal() {
    let f = parse_ok("let a = [1, 2, 3,]\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::Array(items) = &d.init.as_ref().unwrap().kind else {
        panic!()
    };
    assert_eq!(items.len(), 3);
}

// ----- set and map literals -----

/// Extract the initializer expression of the first top-level `let`.
fn let_init(f: &crate::ast::File) -> &ExprKind {
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!("expected a let decl");
    };
    &d.init.as_ref().expect("let initializer").kind
}

#[test]
fn parses_set_literal() {
    let f = parse_ok("let s = {1, 2, 2}\n");
    let ExprKind::SetLit(items) = let_init(&f) else {
        panic!("expected a set literal, got {:?}", let_init(&f));
    };
    assert_eq!(items.len(), 3);
}

#[test]
fn parses_single_element_set_with_trailing_comma() {
    let f = parse_ok("let s = {1,}\n");
    let ExprKind::SetLit(items) = let_init(&f) else {
        panic!("expected a set literal");
    };
    assert_eq!(items.len(), 1);
}

#[test]
fn single_element_brace_is_a_block_not_a_set() {
    // `{ x }` stays a block whose tail expression is `x`, preserving the
    // existing block behavior. A set needs the comma form.
    let f = parse_ok("let a = { 5 }\n");
    let ExprKind::Block(b) = let_init(&f) else {
        panic!("expected a block, not a set");
    };
    assert!(b.stmts.is_empty());
    assert!(matches!(
        b.trailing.as_ref().unwrap().kind,
        ExprKind::Int(5)
    ));
}

#[test]
fn brace_with_statement_is_a_block() {
    let f = parse_ok("let b = { let x = 3; x + 1 }\n");
    assert!(matches!(let_init(&f), ExprKind::Block(_)));
}

#[test]
fn empty_brace_is_a_block() {
    let f = parse_ok("let u = {}\n");
    let ExprKind::Block(b) = let_init(&f) else {
        panic!("expected a block");
    };
    assert!(b.stmts.is_empty());
    assert!(b.trailing.is_none());
}

#[test]
fn parses_map_literal() {
    let f = parse_ok("let m = [\"a\": 1, \"b\": 2]\n");
    let ExprKind::MapLit(pairs) = let_init(&f) else {
        panic!("expected a map literal, got {:?}", let_init(&f));
    };
    assert_eq!(pairs.len(), 2);
}

#[test]
fn empty_map_literal_is_a_map_lit_with_no_pairs() {
    let f = parse_ok("let m = [:]\n");
    let ExprKind::MapLit(pairs) = let_init(&f) else {
        panic!("expected an empty map literal");
    };
    assert!(pairs.is_empty());
}

#[test]
fn empty_bracket_is_an_empty_list() {
    let f = parse_ok("let e = []\n");
    let ExprKind::Array(items) = let_init(&f) else {
        panic!("expected an empty array");
    };
    assert!(items.is_empty());
}

#[test]
fn bracket_without_colon_is_a_list() {
    let f = parse_ok("let l = [1, 2, 3]\n");
    let ExprKind::Array(items) = let_init(&f) else {
        panic!("expected an array");
    };
    assert_eq!(items.len(), 3);
}

#[test]
fn parses_paren_expr() {
    let f = parse_ok("let a = (1 + 2)\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    assert!(matches!(d.init.as_ref().unwrap().kind, ExprKind::Paren(_)));
}

#[test]
fn tuple_expr_is_unsupported() {
    let err = parse_err("let a = (1, 2)\n");
    assert!(
        matches!(err, RavenError::Parse(ParseError::UnsupportedTuple, _, _)),
        "got: {}",
        err
    );
}

#[test]
fn parses_if_expr_with_else() {
    let f = parse_ok("let x = if a { 1 } else { 2 }\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    assert!(matches!(d.init.as_ref().unwrap().kind, ExprKind::If { .. }));
}

#[test]
fn parses_match_expr() {
    let src = "let x = match v {\n  0 -> \"zero\"\n  _ -> \"other\"\n}\n";
    let f = parse_ok(src);
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::Match { arms, .. } = &d.init.as_ref().unwrap().kind else {
        panic!()
    };
    assert_eq!(arms.len(), 2);
    assert!(matches!(
        arms[0].pattern.kind,
        PatternKind::Literal(LiteralPattern::Int(0))
    ));
    assert!(matches!(arms[1].pattern.kind, PatternKind::Wildcard));
}

#[test]
fn parses_negative_i64_min_literal_pattern() {
    // The lexer emits `IntLit(i64::MIN)` for the bare magnitude
    // `9223372036854775808`; negating it with a checked `-` overflowed and
    // panicked the parser in a debug build (issue #531). It must instead
    // wrap to `i64::MIN`.
    let src = "let x = match v {\n  -9223372036854775808 -> \"min\"\n  _ -> \"other\"\n}\n";
    let f = parse_ok(src);
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::Match { arms, .. } = &d.init.as_ref().unwrap().kind else {
        panic!()
    };
    assert!(matches!(
        arms[0].pattern.kind,
        PatternKind::Literal(LiteralPattern::Int(i64::MIN))
    ));
}

#[test]
fn bare_int_min_magnitude_is_out_of_range() {
    // `9223372036854775808` as a positive literal overflows Int; it used to
    // compile as i64::MIN (issue #543). It is now a parse error.
    let err = parse_err("let x = 9223372036854775808\n");
    assert!(matches!(err, RavenError::Parse(_, _, _)), "got: {}", err);
}

#[test]
fn negated_int_min_magnitude_is_i64_min() {
    let f = parse_ok("let x = -9223372036854775808\n");
    assert!(matches!(let_init(&f), ExprKind::Int(i64::MIN)));
}

#[test]
fn parses_while_and_for() {
    let src = "fun f() { while a { let _ = 1 }\nfor x in xs { let _ = 1 }\n }\n";
    let f = parse_ok(src);
    assert_eq!(f.items.len(), 1);
}

#[test]
fn parses_lambda_full_form() {
    let f = parse_ok("let f = fun(x: Int) -> Int { x + 1 }\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::Lambda {
        params,
        params_inferred,
        ..
    } = &d.init.as_ref().unwrap().kind
    else {
        panic!()
    };
    assert_eq!(params.len(), 1);
    assert!(!*params_inferred);
}

#[test]
fn parses_lambda_shorthand() {
    let f = parse_ok("let f = { x -> x * 2 }\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::Lambda {
        params,
        params_inferred,
        ..
    } = &d.init.as_ref().unwrap().kind
    else {
        panic!()
    };
    assert_eq!(params.len(), 1);
    assert!(*params_inferred);
}

#[test]
fn parses_struct_literal() {
    let f = parse_ok("let p = Point { x: 1, y: 2 }\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::StructLit { name, fields, .. } = &d.init.as_ref().unwrap().kind else {
        panic!()
    };
    assert_eq!(name, "Point");
    assert_eq!(fields.len(), 2);
}

#[test]
fn struct_literal_shorthand_field() {
    let f = parse_ok("let p = Point { x, y }\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::StructLit { fields, .. } = &d.init.as_ref().unwrap().kind else {
        panic!()
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "x");
}

#[test]
fn duplicate_field_in_struct_literal_errors() {
    let err = parse_err("let p = Point { x: 1, x: 2 }\n");
    assert!(
        matches!(err, RavenError::Parse(ParseError::DuplicateField(_), _, _)),
        "got: {}",
        err
    );
}

#[test]
fn struct_literal_suppressed_in_if_condition() {
    // The `{ x: 1 }` should be parsed as the body block, not as a
    // struct literal. The condition `a` is just an ident.
    let f = parse_ok("let r = if a { 1 } else { 2 }\n");
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    assert!(matches!(d.init.as_ref().unwrap().kind, ExprKind::If { .. }));
}

// ----- assignment statements -----

#[test]
fn parses_assignment_inside_function() {
    let f = parse_ok("fun f() { let x = 0\n x = 5 }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    let FunctionBody::Block(b) = &fun.body else {
        panic!()
    };
    assert_eq!(b.stmts.len(), 2);
    assert!(matches!(b.stmts[1].kind, StmtKind::Assign { .. }));
}

#[test]
fn parses_const_and_let_stmt_mutability() {
    let f = parse_ok("fun f() { const K = 5\n let m = 1 }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    let FunctionBody::Block(b) = &fun.body else {
        panic!()
    };
    let StmtKind::Let { mutable: k_mut, .. } = &b.stmts[0].kind else {
        panic!("expected a const/let statement")
    };
    let StmtKind::Let { mutable: m_mut, .. } = &b.stmts[1].kind else {
        panic!("expected a let statement")
    };
    assert!(!*k_mut, "const is immutable");
    assert!(*m_mut, "let is mutable");
}

#[test]
fn compound_assignment() {
    let f = parse_ok("fun f() { let x = 0\n x += 1 }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    let FunctionBody::Block(b) = &fun.body else {
        panic!()
    };
    assert!(matches!(b.stmts[1].kind, StmtKind::Assign { .. }));
}

#[test]
fn invalid_assignment_target_errors() {
    let err = parse_err("fun f() { 1 + 2 = 3 }\n");
    assert!(
        matches!(
            err,
            RavenError::Parse(ParseError::InvalidAssignmentTarget, _, _)
        ),
        "got: {}",
        err
    );
}

#[test]
fn call_result_is_not_an_assignment_target() {
    let err = parse_err("fun f() { g() = 3 }\n");
    assert!(
        matches!(
            err,
            RavenError::Parse(ParseError::InvalidAssignmentTarget, _, _)
        ),
        "got: {}",
        err
    );
}

/// Pull the statements out of the first function's block body.
fn fn_body_stmts(f: &crate::ast::File) -> &[crate::ast::Stmt] {
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!("first item is not a function");
    };
    let FunctionBody::Block(b) = &fun.body else {
        panic!("function body is not a block");
    };
    &b.stmts
}

#[test]
fn self_is_a_valid_field_assignment_target() {
    let f = parse_ok("impl T { fun m(self) { self.n = 1 } }\n");
    let DeclKind::Impl(block) = &f.items[0].kind else {
        panic!("first item is not an impl");
    };
    let fun = &block.items[0];
    let FunctionBody::Block(b) = &fun.body else {
        panic!("method body is not a block");
    };
    let StmtKind::Assign { target, .. } = &b.stmts[0].kind else {
        panic!("first statement is not an assignment");
    };
    assert!(matches!(target.kind, ExprKind::Field { .. }));
}

#[test]
fn field_and_index_chains_are_valid_targets() {
    // ident.field, ident[index], and a nested chain on top of both.
    let f = parse_ok("fun f() { a.b = 1\n xs[i] = 2\n obj.items[k] = 3 }\n");
    let stmts = fn_body_stmts(&f);
    assert!(matches!(stmts[0].kind, StmtKind::Assign { .. }));
    assert!(matches!(stmts[1].kind, StmtKind::Assign { .. }));
    assert!(matches!(stmts[2].kind, StmtKind::Assign { .. }));
}

// ----- functions, types, generics -----

#[test]
fn parses_function_with_block_body() {
    let f = parse_ok("fun add(a: Int, b: Int) -> Int { a + b }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(fun.name, "add");
    assert_eq!(fun.params.len(), 2);
    assert!(fun.ret.is_some());
    assert!(matches!(fun.body, FunctionBody::Block(_)));
}

#[test]
fn parses_function_with_expr_body() {
    let f = parse_ok("fun add(a: Int, b: Int) -> Int = a + b\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    assert!(matches!(fun.body, FunctionBody::Expr(_)));
}

#[test]
fn parses_generic_function() {
    let f = parse_ok("fun id<T>(x: T) -> T = x\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(fun.generics.len(), 1);
    assert_eq!(fun.generics[0].name, "T");
}

#[test]
fn parses_generic_with_bounds() {
    let f = parse_ok("fun show<T: Display + Debug>(x: T) { }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(fun.generics[0].bounds.len(), 2);
}

#[test]
fn parses_nested_generic_types() {
    // The closing `>>` must be split.
    let f = parse_ok("fun foo() -> Vec<Vec<Int>> { }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    assert!(fun.ret.is_some());
}

#[test]
fn parses_optional_type_sugar() {
    let f = parse_ok("fun get() -> Int? { }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    let TypeKind::Optional(_) = &fun.ret.as_ref().unwrap().kind else {
        panic!("expected Optional, got {:?}", fun.ret);
    };
}

#[test]
fn parses_function_type() {
    let f = parse_ok("fun apply(f: fun(Int) -> Int) -> Int { f(1) }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    let TypeKind::Function { .. } = &fun.params[0].ty.kind else {
        panic!("expected fun type");
    };
}

#[test]
fn missing_function_body_is_error() {
    let err = parse_err("fun foo() -> Int\n");
    assert!(matches!(err, RavenError::Parse(_, _, _)));
}

// ----- structs, traits, impls, enums -----

#[test]
fn parses_struct_declaration() {
    let f = parse_ok("struct Point { x: Int, y: Int }\n");
    let DeclKind::Struct(s) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(s.fields.len(), 2);
}

#[test]
fn struct_field_newline_separator() {
    let src = "struct Point {\n  x: Int\n  y: Int\n}\n";
    let f = parse_ok(src);
    let DeclKind::Struct(s) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(s.fields.len(), 2);
}

#[test]
fn parses_trait_with_default_method() {
    let f = parse_ok("trait Greet { fun hello() -> String { \"hi\" } }\n");
    let DeclKind::Trait(t) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(t.members.len(), 1);
    assert!(matches!(t.members[0].body, FunctionBody::Block(_)));
}

#[test]
fn parses_trait_with_signature_only() {
    let f = parse_ok("trait Greet { fun hello() -> String\n }\n");
    let DeclKind::Trait(t) = &f.items[0].kind else {
        panic!()
    };
    assert!(matches!(t.members[0].body, FunctionBody::None));
}

#[test]
fn parses_inherent_impl() {
    let f = parse_ok("impl Foo { fun bar() { } }\n");
    let DeclKind::Impl(i) = &f.items[0].kind else {
        panic!()
    };
    assert!(i.for_type.is_none());
    assert_eq!(i.items.len(), 1);
}

#[test]
fn parses_trait_impl() {
    let f = parse_ok("impl Display for Foo { fun show() { } }\n");
    let DeclKind::Impl(i) = &f.items[0].kind else {
        panic!()
    };
    assert!(i.for_type.is_some());
}

#[test]
fn parses_enum_with_variants() {
    let src = "enum Color { Red, Green, Blue }\n";
    let f = parse_ok(src);
    let DeclKind::Enum(e) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(e.variants.len(), 3);
    assert!(matches!(e.variants[0].payload, VariantPayload::Unit));
}

#[test]
fn parses_enum_with_tuple_payload() {
    let src = "enum Result { Ok(Int), Err(String) }\n";
    let f = parse_ok(src);
    let DeclKind::Enum(e) = &f.items[0].kind else {
        panic!()
    };
    let VariantPayload::Tuple(types) = &e.variants[0].payload else {
        panic!()
    };
    assert_eq!(types.len(), 1);
}

#[test]
fn parses_enum_with_struct_payload() {
    let src = "enum Shape { Circle(radius: Int), Square(side: Int) }\n";
    let f = parse_ok(src);
    let DeclKind::Enum(e) = &f.items[0].kind else {
        panic!()
    };
    let VariantPayload::Struct(fields) = &e.variants[0].payload else {
        panic!()
    };
    assert_eq!(fields[0].name, "radius");
}

#[test]
fn derive_attribute_attaches_to_struct() {
    let src = "@derive(Eq, Hash, ToString, Debug)\nstruct Point { x: Int, y: Int }\n";
    let f = parse_ok(src);
    let DeclKind::Struct(s) = &f.items[0].kind else {
        panic!("expected struct decl")
    };
    assert_eq!(s.derives, vec!["Eq", "Hash", "ToString", "Debug"]);
    assert_eq!(s.fields.len(), 2);
}

#[test]
fn derive_attribute_attaches_to_enum() {
    let src = "@derive(Eq, ToString)\nenum Shape { Dot, Circle(Int) }\n";
    let f = parse_ok(src);
    let DeclKind::Enum(e) = &f.items[0].kind else {
        panic!("expected enum decl")
    };
    assert_eq!(e.derives, vec!["Eq", "ToString"]);
    assert_eq!(e.variants.len(), 2);
}

#[test]
fn struct_without_derive_has_empty_list() {
    let f = parse_ok("struct Point { x: Int }\n");
    let DeclKind::Struct(s) = &f.items[0].kind else {
        panic!()
    };
    assert!(s.derives.is_empty());
}

#[test]
fn repr_c_attribute_marks_struct() {
    let src = "@repr(C)\nstruct Point { x: CInt, y: CInt }\n";
    let f = parse_ok(src);
    let DeclKind::Struct(s) = &f.items[0].kind else {
        panic!("expected struct decl")
    };
    assert!(s.repr_c);
    assert!(s.derives.is_empty());
}

#[test]
fn repr_c_and_derive_combine_on_struct() {
    let src = "@repr(C)\n@derive(Eq)\nstruct Point { x: CInt }\n";
    let f = parse_ok(src);
    let DeclKind::Struct(s) = &f.items[0].kind else {
        panic!("expected struct decl")
    };
    assert!(s.repr_c);
    assert_eq!(s.derives, vec!["Eq"]);
}

#[test]
fn struct_without_repr_is_not_repr_c() {
    let f = parse_ok("struct Point { x: Int }\n");
    let DeclKind::Struct(s) = &f.items[0].kind else {
        panic!()
    };
    assert!(!s.repr_c);
}

#[test]
fn unknown_repr_is_a_parse_error() {
    let err = parse_err("@repr(Packed)\nstruct P { x: CInt }\n");
    assert!(matches!(err, RavenError::Parse(_, _, _)), "got: {}", err);
}

#[test]
fn unknown_attribute_is_a_parse_error() {
    let err = parse_err("@inline\nfun f() {}\n");
    assert!(matches!(err, RavenError::Parse(_, _, _)), "got: {}", err);
}

#[test]
fn derive_before_non_type_is_a_parse_error() {
    let err = parse_err("@derive(Eq)\nfun f() {}\n");
    assert!(matches!(err, RavenError::Parse(_, _, _)), "got: {}", err);
}

// ----- imports, externs, const -----

#[test]
fn parses_std_import() {
    let f = parse_ok("import std/io\n");
    let DeclKind::Import(im) = &f.items[0].kind else {
        panic!()
    };
    let ImportSource::Std(parts) = &im.source else {
        panic!()
    };
    assert_eq!(parts, &["io".to_string()]);
}

#[test]
fn parses_std_import_with_selectors() {
    let f = parse_ok("import std/collections { Map, Set }\n");
    let DeclKind::Import(im) = &f.items[0].kind else {
        panic!()
    };
    let names: Vec<&str> = im.selectors.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["Map", "Set"]);
    assert!(im.selectors.iter().all(|s| s.alias.is_none()));
}

#[test]
fn parses_import_selector_renames() {
    let f = parse_ok("import \"./a\" { parse as aparse, Table as MyTable, plain }\n");
    let DeclKind::Import(im) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(im.selectors[0].name, "parse");
    assert_eq!(im.selectors[0].alias.as_deref(), Some("aparse"));
    assert_eq!(im.selectors[0].local(), "aparse");
    assert_eq!(im.selectors[1].name, "Table");
    assert_eq!(im.selectors[1].alias.as_deref(), Some("MyTable"));
    assert_eq!(im.selectors[2].name, "plain");
    assert_eq!(im.selectors[2].alias, None);
    assert_eq!(im.selectors[2].local(), "plain");
}

#[test]
fn parses_quoted_import_with_alias() {
    let f = parse_ok("import \"github.com/x/y\" as http\n");
    let DeclKind::Import(im) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(im.alias.as_deref(), Some("http"));
    let ImportSource::Quoted(s) = &im.source else {
        panic!()
    };
    assert!(s.contains("github.com"));
}

#[test]
fn parses_extern_block() {
    let f = parse_ok("extern \"C\" { fun puts(s: CString) -> Int\n }\n");
    let DeclKind::Extern(e) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(e.abi, "C");
    assert_eq!(e.items.len(), 1);
}

#[test]
fn parses_variadic_extern() {
    let f = parse_ok("extern \"C\" { fun printf(fmt: CStr, ...) -> CInt\n }\n");
    let DeclKind::Extern(e) = &f.items[0].kind else {
        panic!()
    };
    assert!(e.items[0].variadic);
    assert_eq!(e.items[0].params.len(), 1);
    // A signature without `...` is not variadic.
    let g = parse_ok("extern \"C\" { fun abs(x: CInt) -> CInt\n }\n");
    let DeclKind::Extern(e2) = &g.items[0].kind else {
        panic!()
    };
    assert!(!e2.items[0].variadic);
}

#[test]
fn parses_const_decl() {
    let f = parse_ok("const PI: Float = 3.14\n");
    let DeclKind::Const(c) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(c.name, "PI");
}

// ----- block expressions and newline handling -----

#[test]
fn block_with_trailing_expr_is_value_bearing() {
    // The block's last item is a bare expr without separator => trailing.
    let f = parse_ok("fun f() -> Int { 1 + 2 }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    let FunctionBody::Block(b) = &fun.body else {
        panic!()
    };
    assert!(b.trailing.is_some());
    assert_eq!(b.stmts.len(), 0);
}

#[test]
fn block_with_semicolon_terminator_has_no_trailing() {
    let f = parse_ok("fun f() { 1 + 2; }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    let FunctionBody::Block(b) = &fun.body else {
        panic!()
    };
    assert!(b.trailing.is_none());
    assert_eq!(b.stmts.len(), 1);
}

#[test]
fn block_trailing_expr_survives_newline() {
    // Only `;` ends a statement; a trailing newline still leaves the
    // expression as the block's value, matching Rust semantics.
    let f = parse_ok("fun f() -> Int { 1 + 2\n }\n");
    let DeclKind::Function(fun) = &f.items[0].kind else {
        panic!()
    };
    let FunctionBody::Block(b) = &fun.body else {
        panic!()
    };
    assert!(b.trailing.is_some());
    assert_eq!(b.stmts.len(), 0);
}

#[test]
fn newline_inside_expression_is_consumed() {
    // Adding a newline after `+` must not terminate the expression.
    let src = "let x = 1 +\n  2\n";
    let f = parse_ok(src);
    let DeclKind::Let(d) = &f.items[0].kind else {
        panic!()
    };
    assert!(matches!(
        d.init.as_ref().unwrap().kind,
        ExprKind::Binary { .. }
    ));
}

// ----- error variants -----

#[test]
fn unexpected_eof_reports_eof() {
    let err = parse_err("fun foo(");
    assert!(matches!(err, RavenError::Parse(_, _, _)));
}

#[test]
fn invalid_import_path_for_bare_path() {
    let err = parse_err("import 42\n");
    assert!(matches!(err, RavenError::Parse(_, _, _)));
}

// ----- error recovery -----

#[test]
fn recovery_reports_each_broken_statement_in_one_body() {
    // Two statements in the same function body are broken; both errors are
    // reported and the valid statements between and after them still parse.
    let errs = parse_all_errors(
        "fun main() {\n    let a = @\n    let b = 2\n    let c = )\n    let d = 4\n    print(b)\n}\n",
    );
    assert_eq!(errs.len(), 2, "got: {:?}", errs);
}

#[test]
fn statement_recovery_does_not_escape_the_block() {
    // A broken statement in `a` is recovered inside its body; the following
    // function `b` parses cleanly and adds no error.
    let errs = parse_all_errors("fun a() {\n    let x = @\n    let y = 1\n}\nfun b() -> Int = 2\n");
    assert_eq!(errs.len(), 1, "got: {:?}", errs);
}

#[test]
fn recovery_reports_an_error_in_each_broken_item() {
    // Two functions have a broken body; both errors are reported, and the
    // valid functions between and after them do not add noise.
    let errs = parse_all_errors(
        "fun a() -> Int {\n    let x = @\n    return x\n}\nfun b() -> Int { return 2 }\nfun c() -> Int {\n    let y = )\n    return y\n}\nfun d() -> Int { return 4 }\n",
    );
    assert_eq!(errs.len(), 2, "got: {:?}", errs);
}

#[test]
fn recovery_resyncs_at_a_struct_after_a_broken_function() {
    // A broken function followed by a valid struct and function: one error,
    // and recovery finds the next item.
    let errs = parse_all_errors(
        "fun a() -> Int {\n    return @\n}\nstruct P { x: Int }\nfun b() -> Int { return 1 }\n",
    );
    assert_eq!(errs.len(), 1, "got: {:?}", errs);
}

#[test]
fn recovery_on_a_clean_file_reports_nothing() {
    assert!(
        parse_all_errors("fun a() -> Int { return 1 }\nfun b() -> Int { return 2 }\n").is_empty()
    );
}
