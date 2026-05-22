use super::{EnumInfo, StructInfo, Type, TypeChecker};
use crate::ast::{ASTNode, EnumMember, Expression, ImplMember, StructMember};
use std::collections::HashMap;

impl TypeChecker {
    pub fn check(&mut self, node: &ASTNode) -> Result<Type, String> {
        match node {
            ASTNode::VariableDecl(name, expr) => {
                let expr_type = self.check_expression(expr)?;
                self.variables.insert(name.clone(), expr_type.clone());
                Ok(Type::Void)
            }

            ASTNode::VariableDeclTyped(name, type_str, expr) => {
                self.check_variable_decl_typed(name, type_str, expr)
            }

            ASTNode::Assignment(target, expr) => self.check_assignment(target, expr),

            ASTNode::FunctionDecl(name, return_type_str, params, body) => {
                self.check_function_decl(name, return_type_str, params, body)
            }

            ASTNode::StructDecl(name, members) => self.check_struct_decl(name, members),

            ASTNode::ImplBlock(struct_name, methods) => self.check_impl_block(struct_name, methods),

            ASTNode::EnumDecl(name, members) => self.check_enum_decl(name, members),

            ASTNode::Comment(_) => Ok(Type::Void),

            ASTNode::IfStatement(condition, then_block, else_if, else_block) => {
                self.check_if_statement(condition, then_block, else_if, else_block)
            }

            ASTNode::WhileLoop(condition, body) => self.check_while_loop(condition, body),

            ASTNode::ForLoop(init, condition, increment, body) => {
                self.check_for_loop(init, condition, increment, body)
            }

            ASTNode::Block(statements) => {
                for stmt in statements {
                    self.check(stmt)?;
                }
                Ok(Type::Void)
            }

            ASTNode::Print(expr) => {
                self.check_expression(expr)?;
                Ok(Type::Void)
            }

            ASTNode::Return(expr) => self.check_return(expr),

            ASTNode::FunctionCall(name, args) => {
                self.check_expression(&Expression::FunctionCall(name.clone(), args.clone()))?;
                Ok(Type::Void)
            }

            ASTNode::MethodCall(object, method_name, args) => {
                self.check_expression(&Expression::MethodCall(
                    object.clone(),
                    method_name.clone(),
                    args.clone(),
                ))?;
                Ok(Type::Void)
            }

            ASTNode::ExpressionStatement(expr) => {
                self.check_expression(expr)?;
                Ok(Type::Void)
            }

            ASTNode::Import(module_name, alias) => {
                self.load_module_for_type_checking(module_name)?;

                let var_name = alias.as_ref().unwrap_or(module_name);
                self.variables.insert(var_name.clone(), Type::Module);
                self.module_bindings
                    .insert(var_name.clone(), module_name.clone());
                Ok(Type::Void)
            }

            ASTNode::ImportSelective(module_name, items) => {
                self.check_import_selective(module_name, items)
            }

            ASTNode::Export(stmt) => {
                self.check(stmt)?;
                Ok(Type::Void)
            }
        }
    }

    fn check_variable_decl_typed(
        &mut self,
        name: &str,
        type_str: &str,
        expr: &Expression,
    ) -> Result<Type, String> {
        let declared_type = Type::from_string_with_context(type_str, &self.enums, &self.structs);
        let expr_type = self.check_expression_with_expected_type(expr, Some(&declared_type))?;

        if declared_type != expr_type {
            let expected = declared_type.fmt_for_user();
            let got = expr_type.fmt_for_user();
            return Err(format!(
                "Type mismatch in variable '{}': expected {}, got {}\n   = help: Change the expression to match type '{}', or change the variable's declared type to '{}'.",
                name, expected, got, expected, got
            ));
        }

        self.variables.insert(name.to_string(), declared_type);
        Ok(Type::Void)
    }

    fn check_assignment(&mut self, target: &Expression, expr: &Expression) -> Result<Type, String> {
        match target {
            Expression::Identifier(name) => {
                if let Some(var_type) = self.variables.get(name).cloned() {
                    let expr_type =
                        self.check_expression_with_expected_type(expr, Some(&var_type))?;
                    if var_type != expr_type {
                        let expected = var_type.fmt_for_user();
                        let got = expr_type.fmt_for_user();
                        return Err(format!(
                            "Type mismatch in assignment to '{}': expected {}, got {}\n   = help: The value must match the variable's type '{}'. Consider converting or changing the expression.",
                            name, expected, got, expected
                        ));
                    }
                    Ok(Type::Void)
                } else {
                    Err(format!(
                        "Variable '{}' not declared\n   = help: Declare the variable with 'let {}: type = value;' before using it.",
                        name, name
                    ))
                }
            }
            Expression::FieldAccess(object, field_name) => {
                let object_type = self.check_expression(object)?;

                if let Type::Struct(ref struct_name) = object_type {
                    if let Some(struct_info) = self.structs.get(struct_name) {
                        if let Some(field_type) = struct_info.fields.get(field_name) {
                            let field_type = field_type.clone();
                            let expr_type =
                                self.check_expression_with_expected_type(expr, Some(&field_type))?;
                            if expr_type != field_type {
                                let expected = field_type.fmt_for_user();
                                let got = expr_type.fmt_for_user();
                                return Err(format!(
                                    "Type mismatch in assignment to field '{}.{}': expected {}, got {}\n   = help: The value must match the field's type '{}'.",
                                    struct_name, field_name, expected, got, expected
                                ));
                            }
                            return Ok(Type::Void);
                        }
                        let available: Vec<&str> =
                            struct_info.fields.keys().map(String::as_str).collect();
                        return Err(format!(
                            "Field '{}' not found in struct '{}'\n   = help: Available fields: {}",
                            field_name,
                            struct_name,
                            available.join(", ")
                        ));
                    }
                }

                Err(format!(
                    "Cannot assign to field on non-struct value of type '{}'\n   = help: Only struct values have assignable fields.",
                    object_type.fmt_for_user()
                ))
            }
            Expression::ArrayIndex(_, _) => {
                let lhs_type = self.check_expression(target)?;
                let expr_type = self.check_expression_with_expected_type(expr, Some(&lhs_type))?;
                if lhs_type != expr_type {
                    let expected = lhs_type.fmt_for_user();
                    let got = expr_type.fmt_for_user();
                    return Err(format!(
                        "Type mismatch in assignment to indexed value: expected {}, got {}\n   = help: The right-hand side must match the element type at this index (e.g. int for int[][], or int[] for int[][]).",
                        expected, got
                    ));
                }
                Ok(Type::Void)
            }
            _ => {
                self.check_expression(expr)?;
                Ok(Type::Void)
            }
        }
    }

    fn check_function_decl(
        &mut self,
        name: &str,
        return_type_str: &str,
        params: &[crate::ast::Parameter],
        body: &ASTNode,
    ) -> Result<Type, String> {
        let return_type =
            Type::from_string_with_context(return_type_str, &self.enums, &self.structs);

        let param_types: Vec<Type> = params
            .iter()
            .map(|p| Type::from_string_with_context(&p.param_type, &self.enums, &self.structs))
            .collect();

        for (i, param) in params.iter().enumerate() {
            self.variables
                .insert(param.name.clone(), param_types[i].clone());
        }

        self.functions
            .insert(name.to_string(), (return_type.clone(), param_types));

        let saved = self
            .current_function_return_type
            .replace(return_type.clone());
        let check_result = self.check(body);
        self.current_function_return_type = saved;
        check_result?;

        Ok(Type::Void)
    }

    fn check_struct_decl(&mut self, name: &str, members: &[StructMember]) -> Result<Type, String> {
        let mut struct_info = StructInfo {
            fields: HashMap::new(),
        };

        for member in members {
            match member {
                StructMember::Field(field) => {
                    let field_type = Type::from_string_with_context(
                        &field.field_type,
                        &self.enums,
                        &self.structs,
                    );
                    struct_info.fields.insert(field.name.clone(), field_type);
                }
                StructMember::Comment(_) => {}
            }
        }

        self.structs.insert(name.to_string(), struct_info);

        Ok(Type::Void)
    }

    fn check_impl_block(
        &mut self,
        struct_name: &str,
        methods: &[ImplMember],
    ) -> Result<Type, String> {
        if !self.structs.contains_key(struct_name) {
            return Err(format!(
                "Cannot impl for '{}': struct not found\n   = help: Define the struct first with 'struct {} {{ ... }}'",
                struct_name, struct_name
            ));
        }

        for method in methods {
            let (method_name, return_type_str, params, body) = match method {
                ImplMember::Method(n, r, p, b) => (n, r, p, b),
                ImplMember::Comment(_) => continue,
            };
            let return_type =
                Type::from_string_with_context(return_type_str, &self.enums, &self.structs);
            let param_types: Vec<Type> = params
                .iter()
                .map(|p| Type::from_string_with_context(&p.param_type, &self.enums, &self.structs))
                .collect();

            self.struct_methods
                .entry(struct_name.to_string())
                .or_default()
                .insert(
                    method_name.clone(),
                    (return_type.clone(), param_types.clone()),
                );

            let self_type = Type::Struct(struct_name.to_string());
            self.variables.insert("self".to_string(), self_type.clone());

            for (i, param) in params.iter().skip(1).enumerate() {
                if i + 1 < param_types.len() {
                    self.variables
                        .insert(param.name.clone(), param_types[i + 1].clone());
                }
            }

            let saved = self
                .current_function_return_type
                .replace(return_type.clone());
            let check_result = self.check(body);
            self.current_function_return_type = saved;
            check_result?;

            self.variables.remove("self");
            for param in params.iter().skip(1) {
                self.variables.remove(&param.name);
            }
        }

        Ok(Type::Void)
    }

    fn check_enum_decl(&mut self, name: &str, members: &[EnumMember]) -> Result<Type, String> {
        let variants: Vec<String> = members
            .iter()
            .filter_map(|m| match m {
                EnumMember::Variant(v) => Some(v.clone()),
                EnumMember::Comment(_) => None,
            })
            .collect();
        let enum_info = EnumInfo { variants };

        self.enums.insert(name.to_string(), enum_info);

        Ok(Type::Void)
    }

    fn check_if_statement(
        &mut self,
        condition: &Expression,
        then_block: &ASTNode,
        else_if: &Option<Box<ASTNode>>,
        else_block: &Option<Box<ASTNode>>,
    ) -> Result<Type, String> {
        let cond_type = self.check_expression(condition)?;
        if cond_type != Type::Bool {
            return Err(format!(
                "Condition in if statement must be boolean, got {}\n   = help: Use a comparison (e.g. x == 0, a < b) or a boolean variable. Example: if (x > 0) {{ ... }}",
                cond_type.fmt_for_user()
            ));
        }

        self.check(then_block)?;

        if let Some(else_if_node) = else_if {
            self.check(else_if_node)?;
        }

        if let Some(else_node) = else_block {
            self.check(else_node)?;
        }

        Ok(Type::Void)
    }

    fn check_while_loop(&mut self, condition: &Expression, body: &ASTNode) -> Result<Type, String> {
        let cond_type = self.check_expression(condition)?;
        if cond_type != Type::Bool {
            return Err(format!(
                "Condition in while loop must be boolean, got {}\n   = help: Use a comparison (e.g. i < 10) or boolean. Example: while (i < 10) {{ ... }}",
                cond_type.fmt_for_user()
            ));
        }

        self.check(body)?;
        Ok(Type::Void)
    }

    fn check_for_loop(
        &mut self,
        init: &ASTNode,
        condition: &Expression,
        increment: &ASTNode,
        body: &ASTNode,
    ) -> Result<Type, String> {
        self.check(init)?;

        let cond_type = self.check_expression(condition)?;
        if cond_type != Type::Bool {
            return Err(format!(
                "Condition in for loop must be boolean, got {}\n   = help: The condition (middle part) must be a comparison. Example: for (let i: int = 0; i < 10; i = i + 1) {{ ... }}",
                cond_type.fmt_for_user()
            ));
        }

        self.check(increment)?;
        self.check(body)?;

        Ok(Type::Void)
    }

    fn check_return(&mut self, expr: &Expression) -> Result<Type, String> {
        if let Some(expected_rt) = self.current_function_return_type.clone() {
            let actual = self.check_expression_with_expected_type(expr, Some(&expected_rt))?;
            if actual != expected_rt {
                return Err(format!(
                    "Return type mismatch: expected {}, got {}\n   = help: Change the returned expression to match the function's declared return type.",
                    expected_rt.fmt_for_user(),
                    actual.fmt_for_user()
                ));
            }
        } else {
            self.check_expression(expr)?;
        }
        Ok(Type::Void)
    }

    fn check_import_selective(
        &mut self,
        module_name: &str,
        items: &[String],
    ) -> Result<Type, String> {
        self.load_module_for_type_checking(module_name)?;

        if let Some(module) = self.modules.get(module_name) {
            for item in items {
                if let Some(var_type) = module.variables.get(item) {
                    self.variables.insert(item.clone(), var_type.clone());
                } else if let Some((return_type, param_types)) = module.functions.get(item) {
                    self.functions
                        .insert(item.clone(), (return_type.clone(), param_types.clone()));
                } else {
                    return Err(format!(
                        "Item '{}' not found in module '{}'",
                        item, module_name
                    ));
                }
            }
        } else {
            return Err(format!("Module '{}' not found", module_name));
        }
        Ok(Type::Void)
    }
}
