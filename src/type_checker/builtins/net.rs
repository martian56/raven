use super::super::{Type, TypeChecker};
use crate::ast::Expression;

pub(super) fn check(
    tc: &mut TypeChecker,
    name: &str,
    args: &[Expression],
) -> Result<Option<Type>, String> {
    match name {
        "tcp_listen" => {
            if args.len() != 2 {
                return Err(format!(
                    "tcp_listen() expects 2 arguments, got {}",
                    args.len()
                ));
            }
            let a = tc.check_expression(&args[0])?;
            let b = tc.check_expression(&args[1])?;
            if a != Type::String || b != Type::Int {
                return Err(
                    "tcp_listen(addr, backlog) requires string address and int backlog".to_string(),
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
            let t = tc.check_expression(&args[0])?;
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
            let a = tc.check_expression(&args[0])?;
            let b = tc.check_expression(&args[1])?;
            if a != Type::TcpStream || b != Type::Int {
                return Err(
                    "tcp_read(stream, max_bytes) requires TcpStream and int max_bytes".to_string(),
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
            let a = tc.check_expression(&args[0])?;
            let b = tc.check_expression(&args[1])?;
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
            let t = tc.check_expression(&args[0])?;
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

        "dns_lookup" => {
            if args.len() != 1 {
                return Err(format!(
                    "dns_lookup() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let t = tc.check_expression(&args[0])?;
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

            let t = tc.check_expression(&args[0])?;
            if t != Type::String {
                return Err("reachable() hostname must be a string".to_string());
            }

            Ok(Some(Type::Bool))
        }

        _ => Ok(None),
    }
}
