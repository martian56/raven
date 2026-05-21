//! Built-in functions called from eval_expression's FunctionCall arm.

mod core;
mod fs;
mod http;
mod io;
mod net;
mod string;
mod time;

use super::{Interpreter, Value};
use crate::ast::Expression;

impl Interpreter {
    pub(super) fn call_builtin_function(
        &mut self,
        name: &str,
        args: &[Expression],
    ) -> Result<Option<Value>, String> {
        if let Some(v) = core::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = io::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = string::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = time::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = fs::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = net::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = http::call(self, name, args)? {
            return Ok(Some(v));
        }
        Ok(None)
    }
}
