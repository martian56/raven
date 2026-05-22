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
mod tests;
