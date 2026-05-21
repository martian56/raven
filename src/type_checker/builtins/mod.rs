//! Builtin type-checking. Mirrors src/code_gen/builtins/ structure.

mod core;

use super::{Type, TypeChecker};
use crate::ast::Expression;

impl TypeChecker {
    pub(super) fn check_builtin_function(
        &mut self,
        name: &str,
        args: &[Expression],
    ) -> Result<Option<Type>, String> {
        if let Some(t) = core::check(self, name, args)? {
            return Ok(Some(t));
        }

        match name {
            "print" => {
                if args.is_empty() {
                    return Err("print() expects at least 1 argument".to_string());
                }

                for arg in args {
                    self.check_expression(arg)?;
                }

                Ok(Some(Type::Void))
            }

            "input" => {
                if args.len() > 1 {
                    return Err(format!(
                        "input() expects 0 or 1 argument, got {}",
                        args.len()
                    ));
                }

                if args.len() == 1 {
                    let prompt_type = self.check_expression(&args[0])?;
                    if prompt_type != Type::String {
                        return Err("input() prompt must be a string".to_string());
                    }
                }

                Ok(Some(Type::String))
            }

            "read_file" => {
                if args.len() != 1 {
                    return Err(format!(
                        "read_file() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let filename_type = self.check_expression(&args[0])?;
                if filename_type != Type::String {
                    return Err("read_file() filename must be a string".to_string());
                }

                Ok(Some(Type::String))
            }

            "write_file" => {
                if args.len() != 2 {
                    return Err(format!(
                        "write_file() expects 2 arguments, got {}",
                        args.len()
                    ));
                }

                let filename_type = self.check_expression(&args[0])?;
                if filename_type != Type::String {
                    return Err("write_file() filename must be a string".to_string());
                }

                self.check_expression(&args[1])?;

                Ok(Some(Type::Void))
            }

            "append_file" => {
                if args.len() != 2 {
                    return Err(format!(
                        "append_file() expects 2 arguments, got {}",
                        args.len()
                    ));
                }

                let filename_type = self.check_expression(&args[0])?;
                if filename_type != Type::String {
                    return Err("append_file() filename must be a string".to_string());
                }

                self.check_expression(&args[1])?;

                Ok(Some(Type::Void))
            }

            "file_exists" => {
                if args.len() != 1 {
                    return Err(format!(
                        "file_exists() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let filename_type = self.check_expression(&args[0])?;
                if filename_type != Type::String {
                    return Err("file_exists() filename must be a string".to_string());
                }

                Ok(Some(Type::Bool))
            }

            "list_directory" => {
                if args.len() != 1 {
                    return Err(format!(
                        "list_directory() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_type = self.check_expression(&args[0])?;
                if path_type != Type::String {
                    return Err("list_directory() path must be a string".to_string());
                }

                Ok(Some(Type::Array(Box::new(Type::String))))
            }

            "create_directory" | "remove_file" | "remove_directory" => {
                if args.len() != 1 {
                    return Err(format!("{}() expects 1 argument, got {}", name, args.len()));
                }

                let path_type = self.check_expression(&args[0])?;
                if path_type != Type::String {
                    return Err(format!("{}() path must be a string", name));
                }

                Ok(Some(Type::Bool))
            }

            "get_file_size" => {
                if args.len() != 1 {
                    return Err(format!(
                        "get_file_size() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_type = self.check_expression(&args[0])?;
                if path_type != Type::String {
                    return Err("get_file_size() path must be a string".to_string());
                }

                Ok(Some(Type::Int))
            }

            "is_dir" => {
                if args.len() != 1 {
                    return Err(format!("is_dir() expects 1 argument, got {}", args.len()));
                }

                let path_type = self.check_expression(&args[0])?;
                if path_type != Type::String {
                    return Err("is_dir() path must be a string".to_string());
                }

                Ok(Some(Type::Bool))
            }

            "sys_time" | "sys_date" => {
                if !args.is_empty() {
                    return Err(format!(
                        "{}() expects 0 arguments, got {}",
                        name,
                        args.len()
                    ));
                }

                Ok(Some(Type::String))
            }

            "parse_int" => {
                if args.len() != 1 {
                    return Err(format!(
                        "parse_int() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let arg_type = self.check_expression(&args[0])?;
                if arg_type != Type::String {
                    return Err("parse_int() expects a string argument".to_string());
                }

                Ok(Some(Type::Int))
            }

            "char_code" => {
                if args.len() != 1 {
                    return Err(format!(
                        "char_code() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let arg_type = self.check_expression(&args[0])?;
                if arg_type != Type::String {
                    return Err("char_code() expects a string argument".to_string());
                }

                Ok(Some(Type::Int))
            }

            "sys_timestamp" => {
                if !args.is_empty() {
                    return Err(format!(
                        "sys_timestamp() expects 0 arguments, got {}",
                        args.len()
                    ));
                }

                Ok(Some(Type::Float))
            }

            "format" => {
                if args.is_empty() {
                    return Err(format!(
                        "format() expects at least 1 argument, got {}",
                        args.len()
                    ));
                }

                let template_type = self.check_expression(&args[0])?;
                if template_type != Type::String {
                    return Err("format() template must be a string".to_string());
                }

                Ok(Some(Type::String))
            }

            "http_fetch" => {
                if args.len() != 4 {
                    return Err(format!(
                        "http_fetch() expects 4 arguments, got {}",
                        args.len()
                    ));
                }

                let m = self.check_expression(&args[0])?;
                let u = self.check_expression(&args[1])?;
                let h = self.check_expression(&args[2])?;
                let b = self.check_expression(&args[3])?;

                if m != Type::String || u != Type::String || b != Type::String {
                    return Err(
                        "http_fetch(method, url, headers, body) requires string method, url, and body"
                            .to_string(),
                    );
                }

                match h {
                    Type::Array(inner) if *inner == Type::String => {}
                    _ => return Err("http_fetch() headers must be string[]".to_string()),
                }

                Ok(Some(Type::Struct("HttpResponse".to_string())))
            }

            "dns_lookup" => {
                if args.len() != 1 {
                    return Err(format!(
                        "dns_lookup() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let t = self.check_expression(&args[0])?;
                if t != Type::String {
                    return Err("dns_lookup() hostname must be a string".to_string());
                }

                Ok(Some(Type::String))
            }

            "reachable" => {
                if args.len() != 1 {
                    return Err(format!(
                        "reachable() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let t = self.check_expression(&args[0])?;
                if t != Type::String {
                    return Err("reachable() hostname must be a string".to_string());
                }

                Ok(Some(Type::Bool))
            }

            "tcp_listen" => {
                if args.len() != 2 {
                    return Err(format!(
                        "tcp_listen() expects 2 arguments, got {}",
                        args.len()
                    ));
                }
                let a = self.check_expression(&args[0])?;
                let b = self.check_expression(&args[1])?;
                if a != Type::String || b != Type::Int {
                    return Err(
                        "tcp_listen(addr, backlog) requires string address and int backlog"
                            .to_string(),
                    );
                }
                Ok(Some(Type::TcpListener))
            }

            "tcp_accept" => {
                if args.len() != 1 {
                    return Err(format!(
                        "tcp_accept() expects 1 argument, got {}",
                        args.len()
                    ));
                }
                let t = self.check_expression(&args[0])?;
                if t != Type::TcpListener {
                    return Err("tcp_accept() requires a TcpListener".to_string());
                }
                Ok(Some(Type::TcpStream))
            }

            "tcp_read" => {
                if args.len() != 2 {
                    return Err(format!(
                        "tcp_read() expects 2 arguments, got {}",
                        args.len()
                    ));
                }
                let a = self.check_expression(&args[0])?;
                let b = self.check_expression(&args[1])?;
                if a != Type::TcpStream || b != Type::Int {
                    return Err(
                        "tcp_read(stream, max_bytes) requires TcpStream and int max_bytes"
                            .to_string(),
                    );
                }
                Ok(Some(Type::String))
            }

            "tcp_write" => {
                if args.len() != 2 {
                    return Err(format!(
                        "tcp_write() expects 2 arguments, got {}",
                        args.len()
                    ));
                }
                let a = self.check_expression(&args[0])?;
                let b = self.check_expression(&args[1])?;
                if a != Type::TcpStream || b != Type::String {
                    return Err(
                        "tcp_write(stream, data) requires TcpStream and string data".to_string()
                    );
                }
                Ok(Some(Type::Int))
            }

            "tcp_close_stream" | "tcp_close_listener" => {
                if args.len() != 1 {
                    return Err(format!("{}() expects 1 argument, got {}", name, args.len()));
                }
                let t = self.check_expression(&args[0])?;
                let expected = if name == "tcp_close_stream" {
                    Type::TcpStream
                } else {
                    Type::TcpListener
                };
                if t != expected {
                    return Err(format!("{}() requires a {}", name, expected.fmt_for_user()));
                }
                Ok(Some(Type::Void))
            }

            "http_invoke_dispatch" => {
                if args.len() != 1 {
                    return Err(format!(
                        "http_invoke_dispatch() expects 1 argument, got {}",
                        args.len()
                    ));
                }
                let t = self.check_expression(&args[0])?;
                if t != Type::Struct("Request".to_string()) {
                    return Err(
                        "http_invoke_dispatch(req) requires a Request (e.g. from web.read_request)"
                            .to_string(),
                    );
                }
                Ok(Some(Type::String))
            }

            _ => Ok(None),
        }
    }
}
