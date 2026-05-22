use super::super::{Interpreter, Value};

/// Dispatch `module.fn(...)` / `module.var` calls.
///
/// Looks up `method_name` as a function (and then as a variable) on the module
/// named `module_name`, loading the module if it has not been loaded yet. The
/// function path delegates to [`Interpreter::call_function_with_module`].
pub(super) fn call(
    interp: &mut Interpreter,
    module_name: &str,
    method_name: &str,
    evaluated_args: Vec<Value>,
) -> Result<Value, String> {
    if !interp.modules.contains_key(module_name) {
        interp.load_module(module_name)?;
    }
    if let Some(module) = interp.modules.get(module_name) {
        if let Some(func) = module.functions.get(method_name) {
            let func_clone = func.clone();
            let module_clone = module.clone();
            interp.call_function_with_module(&func_clone, evaluated_args, &module_clone)
        } else if let Some(value) = module.variables.get(method_name) {
            Ok(value.clone())
        } else {
            let available: Vec<String> = module
                .functions
                .keys()
                .chain(module.variables.keys())
                .cloned()
                .collect();
            Err(format!(
                "Method '{}' not found in module '{}'\n   = help: Available: {}",
                method_name,
                module_name,
                available.join(", ")
            ))
        }
    } else {
        Err(format!("Module '{}' not found", module_name))
    }
}
