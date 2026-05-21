use super::super::{Type, TypeChecker};
use crate::ast::Expression;

/// Type-check method calls whose receiver is a `Type::Struct(_)`.
pub(super) fn check(
    tc: &mut TypeChecker,
    receiver_type: Type,
    method_name: &str,
    args: &[Expression],
) -> Result<Type, String> {
    let struct_name = match receiver_type {
        Type::Struct(name) => name,
        _ => unreachable!("struct_impl::check called with non-struct receiver type"),
    };

    let (return_type, param_types) = match tc.struct_methods.get(&struct_name) {
        Some(methods) => match methods.get(method_name) {
            Some((ret, params)) => (ret.clone(), params.clone()),
            None => {
                let available: Vec<String> = methods.keys().cloned().collect();
                return Err(format!(
                    "Method '{}' not found on struct '{}'\n   = help: Available methods: {}",
                    method_name,
                    struct_name,
                    available.join(", ")
                ));
            }
        },
        None => {
            return Err(format!(
                "Struct '{}' has no methods defined\n   = help: Add methods with 'impl {} {{ fun method(self, ...) {{ ... }} }}'",
                struct_name, struct_name
            ));
        }
    };
    if args.len() + 1 != param_types.len() {
        return Err(format!(
            "Method '{}' on '{}' expects {} arguments (including self), got {}\n   = help: Expected signature: {}({})",
            method_name,
            struct_name,
            param_types.len() - 1,
            args.len(),
            method_name,
            param_types
                .iter()
                .skip(1)
                .map(|t| t.fmt_for_user())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for (i, arg) in args.iter().enumerate() {
        let arg_type = tc.check_expression(arg)?;
        let expected = &param_types[i + 1];
        if arg_type != *expected {
            return Err(format!(
                "Method '{}' argument {} expects {}, got {}",
                method_name,
                i + 1,
                expected.fmt_for_user(),
                arg_type.fmt_for_user()
            ));
        }
    }
    Ok(return_type)
}
