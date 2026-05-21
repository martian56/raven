use super::{Function, Interpreter, Value};
use crate::ast::{ASTNode, EnumMember, Expression, ImplMember, Parameter, StructMember};
use std::collections::HashMap;

use super::array_ops::flatten_array_index_chain;

impl Interpreter {
    pub fn execute(&mut self, node: &ASTNode) -> Result<Value, String> {
        if self.return_value.is_some() {
            return Ok(self.return_value.clone().unwrap());
        }

        match node {
            ASTNode::VariableDecl(name, expr) => {
                let value = self.eval_expression(expr)?;
                self.variables.insert(name.clone(), value);
                Ok(Value::Void)
            }

            ASTNode::VariableDeclTyped(name, type_str, expr) => {
                self.execute_variable_decl_typed(name, type_str, expr)
            }

            ASTNode::Assignment(target, expr) => self.execute_assignment(target, expr),

            ASTNode::FunctionDecl(name, _return_type, params, body) => {
                self.execute_function_decl(name, params, body)
            }

            ASTNode::StructDecl(name, members) => self.execute_struct_decl(name, members),

            ASTNode::ImplBlock(struct_name, methods) => {
                self.execute_impl_block(struct_name, methods)
            }

            ASTNode::EnumDecl(name, members) => self.execute_enum_decl(name, members),

            ASTNode::Comment(_) => Ok(Value::Void),

            ASTNode::IfStatement(condition, then_block, else_if, else_block) => {
                let cond_value = self.eval_expression(condition)?;

                if let Value::Bool(true) = cond_value {
                    self.execute(then_block)
                } else if let Some(else_if_node) = else_if {
                    self.execute(else_if_node)
                } else if let Some(else_node) = else_block {
                    self.execute(else_node)
                } else {
                    Ok(Value::Void)
                }
            }

            ASTNode::WhileLoop(condition, body) => self.execute_while_loop(condition, body),

            ASTNode::ForLoop(init, condition, increment, body) => {
                self.execute_for_loop(init, condition, increment, body)
            }

            ASTNode::Block(statements) => {
                let mut last_value = Value::Void;
                for stmt in statements {
                    last_value = self.execute(stmt)?;

                    if self.return_value.is_some() {
                        break;
                    }
                }
                Ok(last_value)
            }

            ASTNode::Print(expr) => {
                let value = self.eval_expression(expr)?;
                println!("{}", value);
                Ok(Value::Void)
            }

            ASTNode::FunctionCall(name, args) => {
                if let Some(result) = self.call_builtin_function(name, args)? {
                    return Ok(result);
                }

                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_expression(arg)?);
                }

                self.call_function(name, evaluated_args)
            }

            ASTNode::MethodCall(object, method_name, args) => {
                self.execute_method_call(object, method_name, args)
            }

            ASTNode::ExpressionStatement(expr) => {
                self.eval_expression(expr)?;
                Ok(Value::Void)
            }

            ASTNode::Import(module_name, alias) => {
                self.load_module(module_name)?;

                let var_name = alias.as_ref().unwrap_or(module_name);
                self.variables
                    .insert(var_name.clone(), Value::Module(module_name.clone()));

                Ok(Value::Void)
            }

            ASTNode::ImportSelective(module_name, items) => {
                self.execute_import_selective(module_name, items)
            }

            ASTNode::Export(stmt) => self.execute(stmt),

            ASTNode::Return(expr) => {
                let value = self.eval_expression(expr)?;
                self.return_value = Some(value.clone());
                Ok(value)
            }
        }
    }

    fn execute_variable_decl_typed(
        &mut self,
        name: &str,
        type_str: &str,
        expr: &Expression,
    ) -> Result<Value, String> {
        let value = match expr {
            Expression::Uninitialized => self.default_value_for_type_str(type_str)?,
            _ => self.eval_expression(expr)?,
        };
        self.variables.insert(name.to_string(), value);
        Ok(Value::Void)
    }

    fn execute_assignment(
        &mut self,
        target: &Expression,
        expr: &Expression,
    ) -> Result<Value, String> {
        let value = self.eval_expression(expr)?;

        match target {
            Expression::Identifier(name) => {
                if self.variables.contains_key(name) {
                    self.variables.insert(name.clone(), value);
                    Ok(Value::Void)
                } else {
                    Err(format!("Variable '{}' not declared", name))
                }
            }
            Expression::FieldAccess(object, field_name) => {
                let _object_value = self.eval_expression(object)?;

                match object.as_ref() {
                    Expression::Identifier(var_name) => {
                        if let Some(Value::Struct(_, ref mut fields)) =
                            self.variables.get_mut(var_name)
                        {
                            fields.insert(field_name.clone(), value);
                            Ok(Value::Void)
                        } else {
                            Err(format!("Variable '{}' is not a struct", var_name))
                        }
                    }
                    _ => Err("Cannot assign to complex field expression".to_string()),
                }
            }
            Expression::ArrayIndex(_, _) => {
                if let Some((root, indices)) = flatten_array_index_chain(target) {
                    self.assign_array_flat_target(root, &indices, value)?;
                    Ok(Value::Void)
                } else {
                    Err("Cannot assign to this array expression".to_string())
                }
            }
            _ => Ok(Value::Void),
        }
    }

    fn execute_function_decl(
        &mut self,
        name: &str,
        params: &[Parameter],
        body: &ASTNode,
    ) -> Result<Value, String> {
        self.functions.insert(
            name.to_string(),
            Function {
                params: params.to_vec(),
                body: body.clone(),
            },
        );
        Ok(Value::Void)
    }

    fn execute_struct_decl(
        &mut self,
        name: &str,
        members: &[StructMember],
    ) -> Result<Value, String> {
        let mut field_names = Vec::new();
        let mut types = HashMap::new();
        for m in members {
            match m {
                StructMember::Field(f) => {
                    field_names.push(f.name.clone());
                    types.insert(f.name.clone(), f.field_type.clone());
                }
                StructMember::Comment(_) => {}
            }
        }
        self.struct_field_types.insert(name.to_string(), types);
        self.structs.insert(name.to_string(), field_names);
        Ok(Value::Void)
    }

    fn execute_impl_block(
        &mut self,
        struct_name: &str,
        methods: &[ImplMember],
    ) -> Result<Value, String> {
        for method in methods {
            match method {
                ImplMember::Method(method_name, _return_type, params, body) => {
                    let func = Function {
                        params: params.clone(),
                        body: (**body).clone(),
                    };
                    self.struct_methods
                        .entry(struct_name.to_string())
                        .or_default()
                        .insert(method_name.clone(), func);
                }
                ImplMember::Comment(_) => {}
            }
        }
        Ok(Value::Void)
    }

    fn execute_enum_decl(&mut self, name: &str, members: &[EnumMember]) -> Result<Value, String> {
        let variants: Vec<String> = members
            .iter()
            .filter_map(|m| match m {
                EnumMember::Variant(v) => Some(v.clone()),
                EnumMember::Comment(_) => None,
            })
            .collect();
        self.enums.insert(name.to_string(), variants);
        Ok(Value::Void)
    }

    fn execute_while_loop(
        &mut self,
        condition: &Expression,
        body: &ASTNode,
    ) -> Result<Value, String> {
        loop {
            let cond_value = self.eval_expression(condition)?;

            if let Value::Bool(true) = cond_value {
                self.execute(body)?;

                if self.return_value.is_some() {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(Value::Void)
    }

    fn execute_for_loop(
        &mut self,
        init: &ASTNode,
        condition: &Expression,
        increment: &ASTNode,
        body: &ASTNode,
    ) -> Result<Value, String> {
        self.execute(init)?;

        loop {
            let cond_value = self.eval_expression(condition)?;

            if let Value::Bool(true) = cond_value {
                self.execute(body)?;

                if self.return_value.is_some() {
                    break;
                }

                self.execute(increment)?;
            } else {
                break;
            }
        }
        Ok(Value::Void)
    }

    fn execute_method_call(
        &mut self,
        object: &Expression,
        method_name: &str,
        args: &[Expression],
    ) -> Result<Value, String> {
        let mut evaluated_args = Vec::new();
        for arg in args {
            evaluated_args.push(self.eval_expression(arg)?);
        }

        if let Expression::Identifier(var_name) = object {
            if let Some(Value::Array(mut elements)) = self.variables.get(var_name).cloned() {
                match method_name {
                    "push" => {
                        if evaluated_args.len() != 1 {
                            return Err(format!(
                                "push() expects 1 argument, got {}",
                                evaluated_args.len()
                            ));
                        }
                        elements.push(evaluated_args[0].clone());
                        self.variables
                            .insert(var_name.clone(), Value::Array(elements));
                        Ok(Value::Void)
                    }
                    "pop" => {
                        if !evaluated_args.is_empty() {
                            return Err(format!(
                                "pop() expects 0 arguments, got {}",
                                evaluated_args.len()
                            ));
                        }
                        if elements.is_empty() {
                            return Err("Cannot pop from empty array".to_string());
                        }
                        let popped = elements.pop().unwrap();
                        self.variables
                            .insert(var_name.clone(), Value::Array(elements));
                        Ok(popped)
                    }
                    _ => self.eval_expression(&Expression::MethodCall(
                        Box::new(object.clone()),
                        method_name.to_string(),
                        args.to_vec(),
                    )),
                }
            } else {
                Err(format!("Variable '{}' is not an array", var_name))
            }
        } else {
            self.eval_expression(&Expression::MethodCall(
                Box::new(object.clone()),
                method_name.to_string(),
                args.to_vec(),
            ))
        }
    }

    fn execute_import_selective(
        &mut self,
        module_name: &str,
        items: &[String],
    ) -> Result<Value, String> {
        self.load_module(module_name)?;

        if let Some(module) = self.modules.get(module_name) {
            for item in items {
                if let Some(value) = module.variables.get(item) {
                    self.variables.insert(item.clone(), value.clone());
                } else if let Some(func) = module.functions.get(item) {
                    self.functions.insert(item.clone(), func.clone());
                } else {
                    let available: Vec<String> = module
                        .variables
                        .keys()
                        .chain(module.functions.keys())
                        .cloned()
                        .collect();
                    return Err(format!(
                        "Item '{}' not found in module '{}'\n   = help: Available: {}",
                        item,
                        module_name,
                        available.join(", ")
                    ));
                }
            }
        } else {
            return Err(format!("Module '{}' not found", module_name));
        }

        Ok(Value::Void)
    }
}
