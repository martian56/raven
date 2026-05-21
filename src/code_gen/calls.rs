use super::{Function, Interpreter, Module, Value};

impl Interpreter {
    pub(super) fn call_struct_method(
        &mut self,
        struct_value: Value,
        method_name: &str,
        args: Vec<Value>,
        update_var: Option<String>,
    ) -> Result<Value, String> {
        let (struct_name, _) = match &struct_value {
            Value::Struct(n, f) => (n.clone(), f.clone()),
            _ => {
                return Err("Expected struct value for method call".to_string());
            }
        };

        if let Some(methods) = self.struct_methods.get(&struct_name) {
            if let Some(func) = methods.get(method_name).cloned() {
                let mut full_args = vec![struct_value];
                full_args.extend(args);

                if func.params.len() != full_args.len() {
                    return Err(format!(
                        "Method '{}' expects {} arguments, got {}",
                        method_name,
                        func.params.len(),
                        full_args.len()
                    ));
                }

                let saved_vars = self.variables.clone();
                for (i, param) in func.params.iter().enumerate() {
                    self.variables
                        .insert(param.name.clone(), full_args[i].clone());
                }

                self.return_value = None;
                self.execute(&func.body)?;
                let result = self.return_value.clone().unwrap_or(Value::Void);
                let modified_self = self.variables.get("self").cloned();
                self.return_value = None;
                self.variables = saved_vars;

                if let (Some(var_name), Some(modified)) = (update_var, modified_self) {
                    self.variables.insert(var_name, modified);
                }

                return Ok(result);
            }
        }

        Err(format!(
            "Method '{}' not found on struct '{}'",
            method_name, struct_name
        ))
    }

    pub(super) fn call_function_with_module(
        &mut self,
        func: &Function,
        args: Vec<Value>,
        module: &Module,
    ) -> Result<Value, String> {
        let mut function_variables = self.variables.clone();

        for (name, value) in &module.variables {
            function_variables.insert(name.clone(), value.clone());
        }

        if args.len() != func.params.len() {
            return Err(format!(
                "Function expects {} arguments, got {}",
                func.params.len(),
                args.len()
            ));
        }

        for (i, param) in func.params.iter().enumerate() {
            function_variables.insert(param.name.clone(), args[i].clone());
        }

        let old_variables = std::mem::replace(&mut self.variables, function_variables);
        let old_return_value = self.return_value.take();

        self.return_value = None;
        self.execute(&func.body)?;
        let result = self.return_value.clone().unwrap_or(Value::Void);
        self.return_value = None;

        self.variables = old_variables;
        self.return_value = old_return_value;

        Ok(result)
    }
}
