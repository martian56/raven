use super::super::{Type, TypeChecker};
use crate::ast::Expression;

/// Type-check method calls whose receiver is a `Type::Module`.
///
/// The receiver expression must be an `Expression::Identifier` that binds to a
/// known module; the method name then resolves to an exported module function.
/// Anything that does not match that pattern falls back to `Type::Unknown`,
/// matching the original inline behavior.
pub(super) fn check(
    tc: &mut TypeChecker,
    object_expr: &Expression,
    method_name: &str,
    args: &[Expression],
) -> Result<Type, String> {
    if let Expression::Identifier(module_var) = object_expr {
        if let Some(module_name) = tc.module_bindings.get(module_var) {
            if let Some(module_info) = tc.modules.get(module_name) {
                if let Some((return_type, param_types)) =
                    module_info.functions.get(method_name).cloned()
                {
                    if args.len() != param_types.len() {
                        return Err(format!(
                            "Function '{}.{}' expects {} arguments, got {}\n   = help: Expected signature: {}.{}({})",
                            module_var,
                            method_name,
                            param_types.len(),
                            args.len(),
                            module_var,
                            method_name,
                            param_types
                                .iter()
                                .map(|t| t.fmt_for_user())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }

                    for (i, arg) in args.iter().enumerate() {
                        let arg_type = tc.check_expression(arg)?;
                        if arg_type != param_types[i] {
                            return Err(format!(
                                "Function '{}.{}' parameter {} expects {}, got {}\n   = help: Pass a value of type '{}' for this parameter.",
                                module_var,
                                method_name,
                                i + 1,
                                param_types[i].fmt_for_user(),
                                arg_type.fmt_for_user(),
                                param_types[i].fmt_for_user()
                            ));
                        }
                    }

                    return Ok(return_type);
                } else {
                    let available = module_info
                        .functions
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(format!(
                        "Function '{}.{}' not found\n   = help: Available module functions: {}",
                        module_var,
                        method_name,
                        if available.is_empty() {
                            "(none)".to_string()
                        } else {
                            available
                        }
                    ));
                }
            }
        }
    }
    Ok(Type::Unknown)
}
