use super::{ModuleInfo, TypeChecker};
use std::fs;

impl TypeChecker {
    pub(super) fn load_module_for_type_checking(
        &mut self,
        module_name: &str,
    ) -> Result<(), String> {
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

        let mut module_checker = TypeChecker::new();

        module_checker.check(&ast)?;

        let module_info = ModuleInfo {
            variables: module_checker.variables,
            functions: module_checker.functions.clone(),
        };

        for (name, (return_type, param_types)) in &module_checker.functions {
            self.functions
                .insert(name.clone(), (return_type.clone(), param_types.clone()));
        }

        for (name, struct_info) in &module_checker.structs {
            self.structs.insert(name.clone(), struct_info.clone());
        }

        for (struct_name, methods) in &module_checker.struct_methods {
            for (method_name, (return_type, param_types)) in methods {
                self.struct_methods
                    .entry(struct_name.clone())
                    .or_default()
                    .insert(
                        method_name.clone(),
                        (return_type.clone(), param_types.clone()),
                    );
            }
        }

        self.modules.insert(module_name.to_string(), module_info);

        for (nested_name, nested_info) in module_checker.modules {
            self.modules.entry(nested_name).or_insert(nested_info);
        }

        Ok(())
    }
}
