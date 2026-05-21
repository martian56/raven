use super::{Interpreter, Value};
use crate::ast::Expression;

pub(super) fn flatten_array_index_chain(
    expr: &Expression,
) -> Option<(&Expression, Vec<&Expression>)> {
    let mut indices = Vec::new();
    let mut e = expr;
    loop {
        match e {
            Expression::ArrayIndex(array_expr, index_expr) => {
                indices.push(index_expr.as_ref());
                e = array_expr.as_ref();
            }
            Expression::Identifier(_) | Expression::FieldAccess(..) => {
                indices.reverse();
                return Some((e, indices));
            }
            _ => return None,
        }
    }
}

pub(super) fn assign_array_element_by_path(
    value: &mut Value,
    indices: &[usize],
    final_value: Value,
) -> Result<(), String> {
    if indices.is_empty() {
        return Err("Array assignment requires at least one index".to_string());
    }

    let mut current = value;
    for (depth, &idx) in indices.iter().enumerate() {
        if depth == indices.len() - 1 {
            match current {
                Value::Array(elements) => {
                    if idx < elements.len() {
                        elements[idx] = final_value;
                        return Ok(());
                    }
                    return Err(format!(
                        "Array index {} out of bounds (array length: {})",
                        idx,
                        elements.len()
                    ));
                }
                _ => return Err("Cannot assign through non-array value".to_string()),
            }
        } else {
            match current {
                Value::Array(elements) => {
                    if idx < elements.len() {
                        current = &mut elements[idx];
                    } else {
                        return Err(format!(
                            "Array index {} out of bounds (array length: {})",
                            idx,
                            elements.len()
                        ));
                    }
                }
                _ => return Err("Cannot index non-array value".to_string()),
            }
        }
    }
    unreachable!()
}

impl Interpreter {
    pub(super) fn assign_array_flat_target(
        &mut self,
        root: &Expression,
        indices: &[&Expression],
        value: Value,
    ) -> Result<(), String> {
        let index_vals: Vec<usize> = indices
            .iter()
            .map(|e| match self.eval_expression(e)? {
                Value::Int(i) if i >= 0 => Ok(i as usize),
                _ => Err("Array index must be a non-negative integer".to_string()),
            })
            .collect::<Result<Vec<_>, String>>()?;

        match root {
            Expression::Identifier(name) => {
                if let Some(v) = self.variables.get_mut(name) {
                    assign_array_element_by_path(v, &index_vals, value)
                } else {
                    Err(format!("Variable '{}' not declared", name))
                }
            }
            Expression::FieldAccess(obj, field) => match obj.as_ref() {
                Expression::Identifier(obj_name) => {
                    if let Some(Value::Struct(_, ref mut fields)) = self.variables.get_mut(obj_name)
                    {
                        if let Some(v) = fields.get_mut(field) {
                            assign_array_element_by_path(v, &index_vals, value)
                        } else {
                            Err(format!("Field '{}' not found on struct", field))
                        }
                    } else {
                        Err(format!("Variable '{}' is not a struct", obj_name))
                    }
                }
                _ => Err("Cannot assign to complex field array expression".to_string()),
            },
            _ => Err("Invalid array assignment target".to_string()),
        }
    }
}
