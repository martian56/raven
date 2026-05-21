//! Builtin type-checking. Mirrors src/code_gen/builtins/ structure.

mod core;
mod fs;
mod io;
mod string;
mod time;

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
        if let Some(t) = io::check(self, name, args)? {
            return Ok(Some(t));
        }
        if let Some(t) = string::check(self, name, args)? {
            return Ok(Some(t));
        }
        if let Some(t) = time::check(self, name, args)? {
            return Ok(Some(t));
        }
        if let Some(t) = fs::check(self, name, args)? {
            return Ok(Some(t));
        }

        match name {
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
