use std::collections::HashMap;

mod binop;
mod builtins;
mod expr;
mod methods;
mod modules;
mod stmt;
mod type_repr;
pub use type_repr::Type;

pub struct TypeChecker {
    variables: HashMap<String, Type>,
    module_bindings: HashMap<String, String>,
    functions: HashMap<String, (Type, Vec<Type>)>,
    structs: HashMap<String, StructInfo>,
    struct_methods: HashMap<String, HashMap<String, (Type, Vec<Type>)>>,
    enums: HashMap<String, EnumInfo>,
    modules: HashMap<String, ModuleInfo>,
    current_function_return_type: Option<Type>,
}

#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub variables: HashMap<String, Type>,
    pub functions: HashMap<String, (Type, Vec<Type>)>,
}

#[derive(Debug, Clone)]
pub struct StructInfo {
    pub fields: HashMap<String, Type>,
}

pub struct EnumInfo {
    pub variants: Vec<String>,
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut structs = HashMap::new();
        let mut http_response_fields = HashMap::new();
        http_response_fields.insert("status_code".to_string(), Type::Int);
        http_response_fields.insert("status_text".to_string(), Type::String);
        http_response_fields.insert("headers".to_string(), Type::Array(Box::new(Type::String)));
        http_response_fields.insert("body".to_string(), Type::String);
        structs.insert(
            "HttpResponse".to_string(),
            StructInfo {
                fields: http_response_fields,
            },
        );

        TypeChecker {
            variables: HashMap::new(),
            module_bindings: HashMap::new(),
            functions: HashMap::new(),
            structs,
            struct_methods: HashMap::new(),
            enums: HashMap::new(),
            modules: HashMap::new(),
            current_function_return_type: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ASTNode, Expression};
    use std::collections::HashMap;

    #[test]
    fn test_uninitialized_struct_declaration() {
        let mut checker = TypeChecker::new();
        let mut fields = HashMap::new();
        fields.insert("x".to_string(), Type::Int);
        fields.insert("y".to_string(), Type::Int);
        checker
            .structs
            .insert("Point".to_string(), StructInfo { fields });

        let node = ASTNode::VariableDeclTyped(
            "p".to_string(),
            "Point".to_string(),
            Box::new(Expression::Uninitialized),
        );
        assert!(checker.check(&node).is_ok());
    }

    #[test]
    fn test_uninitialized_only_for_struct_type() {
        let mut checker = TypeChecker::new();
        let node = ASTNode::VariableDeclTyped(
            "x".to_string(),
            "int".to_string(),
            Box::new(Expression::Uninitialized),
        );
        assert!(checker.check(&node).is_err());
    }

    #[test]
    fn test_variable_declaration() {
        let mut checker = TypeChecker::new();
        let node = ASTNode::VariableDeclTyped(
            "x".to_string(),
            "int".to_string(),
            Box::new(Expression::Integer(5)),
        );

        assert!(checker.check(&node).is_ok());
    }

    #[test]
    fn test_type_mismatch() {
        let mut checker = TypeChecker::new();
        let node = ASTNode::VariableDeclTyped(
            "x".to_string(),
            "int".to_string(),
            Box::new(Expression::StringLiteral("hello".to_string())),
        );

        assert!(checker.check(&node).is_err());
    }

    #[test]
    fn test_undeclared_variable() {
        let mut checker = TypeChecker::new();
        let expr = Expression::Identifier("x".to_string());

        assert!(checker.check_expression(&expr).is_err());
    }

    #[test]
    fn test_empty_nested_array_literal_with_expected_matrix_type() {
        let mut checker = TypeChecker::new();
        let node = ASTNode::VariableDeclTyped(
            "m".to_string(),
            "int[][]".to_string(),
            Box::new(Expression::ArrayLiteral(vec![
                Expression::ArrayLiteral(vec![]),
                Expression::ArrayLiteral(vec![]),
            ])),
        );
        assert!(checker.check(&node).is_ok());
    }

    #[test]
    fn test_module_method_call_resolves_return_type() {
        let mut checker = TypeChecker::new();
        checker.variables.insert("json".to_string(), Type::Module);
        checker
            .module_bindings
            .insert("json".to_string(), "json".to_string());

        let mut module_functions = HashMap::new();
        module_functions.insert("load".to_string(), (Type::String, vec![Type::String]));
        checker.modules.insert(
            "json".to_string(),
            ModuleInfo {
                variables: HashMap::new(),
                functions: module_functions,
            },
        );

        let expr = Expression::MethodCall(
            Box::new(Expression::Identifier("json".to_string())),
            "load".to_string(),
            vec![Expression::StringLiteral("test.json".to_string())],
        );

        assert_eq!(checker.check_expression(&expr).unwrap(), Type::String);
    }

    #[test]
    fn tcp_builtins_typecheck_like_network_wrappers() {
        let src = r#"
export fun listen(addr: string, backlog: int) -> TcpListener {
    return tcp_listen(addr, backlog);
}

export fun accept(listener: TcpListener) -> TcpStream {
    return tcp_accept(listener);
}

export fun close_listener(listener: TcpListener) -> void {
    tcp_close_listener(listener);
}
"#;
        let lexer = crate::lexer::Lexer::new(src.to_string());
        let mut parser = crate::parser::Parser::new(lexer, src.to_string());
        let ast = parser.parse().expect("parse tcp wrapper module");
        let mut checker = TypeChecker::new();
        checker
            .check(&ast)
            .expect("typecheck tcp builtins in functions");
    }
}
