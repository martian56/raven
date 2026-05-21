use super::{EnumInfo, StructInfo};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    Void,
    Array(Box<Type>),
    Struct(String),
    Enum(String),
    Module,
    Unknown,
    TcpListener,
    TcpStream,
}

impl Type {
    pub fn fmt_for_user(&self) -> String {
        match self {
            Type::Int => "int".to_string(),
            Type::Float => "float".to_string(),
            Type::Bool => "bool".to_string(),
            Type::String => "string".to_string(),
            Type::Void => "void".to_string(),
            Type::Array(inner) => format!("{}[]", inner.fmt_for_user()),
            Type::Struct(s) => s.clone(),
            Type::Enum(s) => s.clone(),
            Type::Module => "module".to_string(),
            Type::Unknown => "unknown".to_string(),
            Type::TcpListener => "TcpListener".to_string(),
            Type::TcpStream => "TcpStream".to_string(),
        }
    }

    pub fn from_string(s: &str) -> Type {
        if let Some(inner) = s.strip_suffix("[]") {
            if inner.is_empty() {
                return Type::Struct(s.to_string());
            }
            return Type::Array(Box::new(Type::from_string(inner)));
        }
        match s {
            "int" => Type::Int,
            "float" => Type::Float,
            "bool" => Type::Bool,
            "string" => Type::String,
            "void" => Type::Void,
            "TcpListener" => Type::TcpListener,
            "TcpStream" => Type::TcpStream,
            _ => Type::Struct(s.to_string()),
        }
    }

    pub fn from_string_with_context(
        s: &str,
        enums: &HashMap<String, EnumInfo>,
        structs: &HashMap<String, StructInfo>,
    ) -> Type {
        if let Some(inner) = s.strip_suffix("[]") {
            if inner.is_empty() {
                return Type::Struct(s.to_string());
            }
            return Type::Array(Box::new(Type::from_string_with_context(
                inner, enums, structs,
            )));
        }
        match s {
            "int" => Type::Int,
            "float" => Type::Float,
            "bool" => Type::Bool,
            "string" => Type::String,
            "void" => Type::Void,
            "TcpListener" => Type::TcpListener,
            "TcpStream" => Type::TcpStream,
            _ => {
                if enums.contains_key(s) {
                    Type::Enum(s.to_string())
                } else {
                    Type::Struct(s.to_string())
                }
            }
        }
    }
}
