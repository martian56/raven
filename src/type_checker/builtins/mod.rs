//! Builtin type-checking. Mirrors src/code_gen/builtins/ structure.

mod core;
mod fs;
mod io;
mod net;
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
        if let Some(t) = net::check(self, name, args)? {
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
