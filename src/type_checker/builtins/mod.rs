//! Builtin type-checking. Mirrors src/code_gen/builtins/ structure.

mod core;
mod fs;
mod http;
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
        if let Some(t) = http::check(self, name, args)? {
            return Ok(Some(t));
        }

        match name {
            _ => Ok(None),
        }
    }
}
