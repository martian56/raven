mod array;
mod module;
mod string;
mod struct_impl;
mod tcp;

use super::{Type, TypeChecker};
use crate::ast::Expression;

impl TypeChecker {
    pub(super) fn check_method_call(
        &mut self,
        object_expr: &Expression,
        method_name: &str,
        args: &[Expression],
    ) -> Result<Type, String> {
        let object_type = self.check_expression(object_expr)?;

        if let Type::Array(_) = object_type {
            array::check(self, object_type, method_name, args)
        } else if let Type::Module = object_type {
            module::check(self, object_expr, method_name, args)
        } else if let Type::String = object_type {
            string::check(self, Type::String, method_name, args)
        } else if matches!(object_type, Type::TcpListener | Type::TcpStream) {
            tcp::check(self, object_type, method_name, args)
        } else if let Type::Struct(_) = object_type {
            struct_impl::check(self, object_type, method_name, args)
        } else {
            Err(format!(
                "Cannot call method on value of type '{}'\n   = help: Methods work on arrays, strings, modules, and structs with impl blocks.",
                object_type.fmt_for_user()
            ))
        }
    }
}
