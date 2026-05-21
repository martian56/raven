use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<Value>),
    Struct(String, HashMap<String, Value>),
    Enum(String, String),
    Module(String),
    Void,
    TcpListener(u64),
    TcpStream(u64),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::Bool(b) => write!(f, "{}", b),
            Value::String(s) => write!(f, "{}", s),
            Value::Array(elements) => {
                write!(f, "[")?;
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", element)?;
                }
                write!(f, "]")
            }
            Value::Struct(name, fields) => {
                write!(f, "{} {{", name)?;
                for (i, (field_name, field_value)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", field_name, field_value)?;
                }
                write!(f, "}}")
            }
            Value::Enum(enum_name, variant_name) => {
                write!(f, "{}::{}", enum_name, variant_name)
            }
            Value::Module(name) => write!(f, "<module: {}>", name),
            Value::Void => write!(f, "void"),
            Value::TcpListener(id) => write!(f, "<TcpListener {}>", id),
            Value::TcpStream(id) => write!(f, "<TcpStream {}>", id),
        }
    }
}
