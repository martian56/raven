use super::*;
use crate::ast::Expression;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::type_checker::TypeChecker;

#[test]
fn test_struct_field_array_push_updates_struct() {
    let src = r#"
struct S { items: string[] }
let s: S = S { items: [] };
s.items.push("a");
"#;
    let lexer = Lexer::new(src.to_string());
    let mut parser = Parser::new(lexer, src.to_string());
    let ast = parser.parse().expect("parse");
    let mut checker = TypeChecker::new();
    checker.check(&ast).expect("typecheck");
    let mut interp = Interpreter::new();
    interp.execute(&ast).expect("run");

    let Some(Value::Struct(_, fields)) = interp.variables.get("s") else {
        panic!("expected s");
    };
    let Some(Value::Array(items)) = fields.get("items") else {
        panic!("expected items array");
    };
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0], Value::String(ref t) if t == "a"));
}

#[test]
fn test_nested_array_assignment_and_read() {
    let src = r#"
let m: int[][] = [[1, 2], [3, 4]];
m[0][1] = 9;
let m2: int[][][] = [[[1]], [[2]]];
m2[0][0][0] = 7;
"#;
    let lexer = Lexer::new(src.to_string());
    let mut parser = Parser::new(lexer, src.to_string());
    let ast = parser.parse().expect("parse");
    let mut checker = TypeChecker::new();
    checker.check(&ast).expect("typecheck");
    let mut interp = Interpreter::new();
    interp.execute(&ast).expect("run");

    if let Some(Value::Array(rows)) = interp.variables.get("m") {
        assert_eq!(rows.len(), 2);
        if let Value::Array(r0) = &rows[0] {
            assert!(matches!(r0[1], Value::Int(9)));
        } else {
            panic!("expected row 0 to be array");
        }
    } else {
        panic!("expected m");
    }

    if let Some(Value::Array(planes)) = interp.variables.get("m2") {
        if let Value::Array(rows) = &planes[0] {
            if let Value::Array(cells) = &rows[0] {
                assert!(matches!(cells[0], Value::Int(7)));
            } else {
                panic!("expected depth 3");
            }
        } else {
            panic!("expected depth 2");
        }
    } else {
        panic!("expected m2");
    }
}

#[test]
fn test_variable_assignment() {
    let mut interp = Interpreter::new();
    let node = ASTNode::VariableDeclTyped(
        "x".to_string(),
        "int".to_string(),
        Box::new(Expression::Integer(42)),
    );

    assert!(interp.execute(&node).is_ok());
    assert_eq!(interp.variables.get("x").unwrap().to_string(), "42");
}

#[test]
fn test_arithmetic() {
    let mut interp = Interpreter::new();
    let expr = Expression::BinaryOp(
        Box::new(Expression::Integer(10)),
        Operator::Add,
        Box::new(Expression::Integer(5)),
    );

    let result = interp.eval_expression(&expr).unwrap();
    if let Value::Int(v) = result {
        assert_eq!(v, 15);
    } else {
        panic!("Expected integer result");
    }
}

#[test]
fn test_print() {
    let mut interp = Interpreter::new();
    let node = ASTNode::Print(Box::new(Expression::StringLiteral(
        "Hello, Raven!".to_string(),
    )));

    assert!(interp.execute(&node).is_ok());
}
