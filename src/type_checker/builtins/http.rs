use super::super::{Type, TypeChecker};
use crate::ast::Expression;

pub(super) fn check(
    tc: &mut TypeChecker,
    name: &str,
    args: &[Expression],
) -> Result<Option<Type>, String> {
    match name {
        "http_fetch" => {
            if args.len() != 4 {
                return Err(format!(
                    "http_fetch() expects 4 arguments, got {}",
                    args.len()
                ));
            }

            let m = tc.check_expression(&args[0])?;
            let u = tc.check_expression(&args[1])?;
            let h = tc.check_expression(&args[2])?;
            let b = tc.check_expression(&args[3])?;

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
            let t = tc.check_expression(&args[0])?;
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
