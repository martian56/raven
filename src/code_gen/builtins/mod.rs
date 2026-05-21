//! Built-in functions called from eval_expression's FunctionCall arm.

mod core;
mod fs;
mod io;
mod net;
mod string;
mod time;

use super::{Interpreter, Value};
use crate::ast::Expression;
use std::collections::HashMap;

impl Interpreter {
    fn value_from_ureq_response(resp: ureq::Response) -> Result<Value, String> {
        let status = resp.status() as i64;
        let status_text = resp.status_text().to_string();
        let mut header_strings: Vec<Value> = Vec::new();
        for name in resp.headers_names() {
            if let Some(v) = resp.header(&name) {
                header_strings.push(Value::String(format!("{}: {}", name, v)));
            }
        }
        let body_str = resp.into_string().unwrap_or_default();
        let mut fields = HashMap::new();
        fields.insert("status_code".to_string(), Value::Int(status));
        fields.insert("status_text".to_string(), Value::String(status_text));
        fields.insert("headers".to_string(), Value::Array(header_strings));
        fields.insert("body".to_string(), Value::String(body_str));
        Ok(Value::Struct("HttpResponse".to_string(), fields))
    }

    pub(super) fn call_builtin_function(
        &mut self,
        name: &str,
        args: &[Expression],
    ) -> Result<Option<Value>, String> {
        if let Some(v) = core::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = io::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = string::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = time::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = fs::call(self, name, args)? {
            return Ok(Some(v));
        }
        if let Some(v) = net::call(self, name, args)? {
            return Ok(Some(v));
        }
        match name {
            "http_fetch" => {
                if args.len() != 4 {
                    return Err(format!(
                        "http_fetch() expects 4 arguments, got {}",
                        args.len()
                    ));
                }

                let method_val = self.eval_expression(&args[0])?;
                let url_val = self.eval_expression(&args[1])?;
                let headers_val = self.eval_expression(&args[2])?;
                let body_val = self.eval_expression(&args[3])?;

                let method = match method_val {
                    Value::String(s) => s,
                    _ => return Err("http_fetch() method must be a string".to_string()),
                };
                let url = match url_val {
                    Value::String(s) => s,
                    _ => return Err("http_fetch() url must be a string".to_string()),
                };
                let body = match body_val {
                    Value::String(s) => s,
                    other => other.to_string(),
                };

                let headers_vec: Vec<String> = match headers_val {
                    Value::Array(elements) => {
                        let mut v = Vec::new();
                        for e in elements {
                            match e {
                                Value::String(s) => v.push(s),
                                _ => {
                                    return Err("http_fetch() headers must be an array of strings"
                                        .to_string());
                                }
                            }
                        }
                        v
                    }
                    _ => return Err("http_fetch() headers must be string[]".to_string()),
                };

                let agent = ureq::Agent::new();
                let mut req = agent.request(method.trim(), &url).set(
                    "User-Agent",
                    "Raven/1.4 (+https://github.com/martian56/raven)",
                );
                for h in &headers_vec {
                    if let Some(colon) = h.find(':') {
                        let hn = h[..colon].trim();
                        let hv = h[colon + 1..].trim();
                        if !hn.is_empty() {
                            req = req.set(hn, hv);
                        }
                    }
                }

                let resp_result = if body.is_empty() {
                    req.call()
                } else {
                    req.send_string(&body)
                };

                match resp_result {
                    Ok(resp) => Ok(Some(Self::value_from_ureq_response(resp)?)),
                    Err(ureq::Error::Status(_code, resp)) => {
                        Ok(Some(Self::value_from_ureq_response(resp)?))
                    }
                    Err(ureq::Error::Transport(e)) => {
                        let mut fields = HashMap::new();
                        fields.insert("status_code".to_string(), Value::Int(0));
                        fields.insert(
                            "status_text".to_string(),
                            Value::String("Transport Error".to_string()),
                        );
                        fields.insert("headers".to_string(), Value::Array(vec![]));
                        fields.insert("body".to_string(), Value::String(e.to_string()));
                        Ok(Some(Value::Struct("HttpResponse".to_string(), fields)))
                    }
                }
            }

            "http_invoke_dispatch" => {
                if args.len() != 1 {
                    return Err(format!(
                        "http_invoke_dispatch() expects 1 argument, got {}",
                        args.len()
                    ));
                }
                let req = self.eval_expression(&args[0])?;
                match &req {
                    Value::Struct(name, _) if name == "Request" => {}
                    _ => {
                        return Err(format!(
                            "http_invoke_dispatch(req) requires a Request value, got {:?}",
                            req
                        ));
                    }
                }
                let result = self.call_function("dispatch", vec![req])?;
                match result {
                    Value::String(s) => Ok(Some(Value::String(s))),
                    other => Err(format!(
                        "dispatch() must return the full HTTP response as a string; got {:?}",
                        other
                    )),
                }
            }

            _ => Ok(None),
        }
    }
}
