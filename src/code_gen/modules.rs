use super::{Interpreter, Module};
use std::fs;

impl Interpreter {
    pub(super) fn load_module(&mut self, module_name: &str) -> Result<(), String> {
        if self.modules.contains_key(module_name) {
            return Ok(());
        }

        let module_path = crate::paths::resolve_module_path(module_name);

        let content = fs::read_to_string(&module_path)
            .map_err(|e| format!("Failed to load module '{}': {}", module_path, e))?;

        let lexer = crate::lexer::Lexer::new(content.clone());
        let mut parser = crate::parser::Parser::new(lexer, content);
        let ast = parser.parse().map_err(|e| {
            format!(
                "Failed to parse module '{}': {}",
                module_path,
                e.with_filename(module_path.clone()).format()
            )
        })?;

        let mut module_interpreter = Interpreter::new();

        module_interpreter.execute(&ast)?;

        let nested_modules_snapshot = module_interpreter.modules.clone();

        let module = Module {
            variables: module_interpreter.variables,
            functions: module_interpreter.functions,
            exports: Vec::new(),
        };

        for (name, func) in &module.functions {
            self.functions.insert(name.clone(), func.clone());
        }

        for (name, fields) in &module_interpreter.structs {
            self.structs.insert(name.clone(), fields.clone());
        }

        for (name, types) in &module_interpreter.struct_field_types {
            self.struct_field_types.insert(name.clone(), types.clone());
        }

        for (struct_name, methods) in &module_interpreter.struct_methods {
            for (method_name, func) in methods {
                self.struct_methods
                    .entry(struct_name.clone())
                    .or_default()
                    .insert(method_name.clone(), func.clone());
            }
        }

        self.modules.insert(module_name.to_string(), module);

        for (nested_name, nested_mod) in nested_modules_snapshot {
            self.modules.entry(nested_name).or_insert(nested_mod);
        }

        Ok(())
    }
}
