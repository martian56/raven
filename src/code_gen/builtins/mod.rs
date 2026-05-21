//! Built-in functions called from eval_expression's FunctionCall arm.

use super::{Interpreter, Value};
use crate::ast::Expression;
use chrono::Local;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

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
        match name {
            "len" => {
                if args.len() != 1 {
                    return Err(format!("len() expects 1 argument, got {}", args.len()));
                }

                let value = self.eval_expression(&args[0])?;
                match value {
                    Value::Array(elements) => Ok(Some(Value::Int(elements.len() as i64))),
                    Value::String(s) => Ok(Some(Value::Int(s.len() as i64))),
                    _ => Err(format!("len() expects array or string, got {:?}", value)),
                }
            }

            "type" => {
                if args.len() != 1 {
                    return Err(format!("type() expects 1 argument, got {}", args.len()));
                }

                let value = self.eval_expression(&args[0])?;
                let type_name = match value {
                    Value::Int(_) => "int".to_string(),
                    Value::Float(_) => "float".to_string(),
                    Value::Bool(_) => "bool".to_string(),
                    Value::String(_) => "string".to_string(),
                    Value::Array(_) => "array".to_string(),
                    Value::Struct(name, _) => name.clone(),
                    Value::Enum(name, _) => name.clone(),
                    Value::Module(_) => "module".to_string(),
                    Value::Void => "void".to_string(),
                    Value::TcpListener(_) => "TcpListener".to_string(),
                    Value::TcpStream(_) => "TcpStream".to_string(),
                };
                Ok(Some(Value::String(type_name.to_string())))
            }

            "print" => {
                if args.is_empty() {
                    return Err("print() expects at least 1 argument".to_string());
                }

                if args.len() == 1 {
                    let value = self.eval_expression(&args[0])?;
                    println!("{}", value);
                } else {
                    let format_string = self.eval_expression(&args[0])?;
                    if let Value::String(format_str) = format_string {
                        let mut formatted = format_str.clone();

                        for (i, arg) in args.iter().enumerate().skip(1) {
                            let arg_value = self.eval_expression(arg)?;
                            let placeholder = "{}";
                            if let Some(pos) = formatted.find(placeholder) {
                                formatted.replace_range(
                                    pos..pos + placeholder.len(),
                                    &arg_value.to_string(),
                                );
                            } else {
                                return Err(format!("Too many arguments for print() - format string has no placeholder for argument {}", i));
                            }
                        }

                        if formatted.contains("{}") {
                            return Err("Too few arguments for print() - format string has unmatched placeholders".to_string());
                        }

                        println!("{}", formatted);
                    } else {
                        return Err("print() format string must be a string".to_string());
                    }
                }

                Ok(Some(Value::Void))
            }

            "panic" => {
                if args.is_empty() {
                    return Err("panic() expects at least 1 argument".to_string());
                }
                let mut parts = Vec::new();
                for arg in args {
                    let v = self.eval_expression(arg)?;
                    parts.push(v.to_string());
                }
                Err(parts.join(""))
            }

            "input" => {
                use std::io::{self, Write};

                if args.len() > 1 {
                    return Err(format!(
                        "input() expects 0 or 1 argument, got {}",
                        args.len()
                    ));
                }

                if args.len() == 1 {
                    let prompt = self.eval_expression(&args[0])?;
                    if let Value::String(prompt_str) = prompt {
                        print!("{}", prompt_str);
                        io::stdout().flush().unwrap();
                    } else {
                        return Err("input() prompt must be a string".to_string());
                    }
                }

                let mut input = String::new();
                match io::stdin().read_line(&mut input) {
                    Ok(_) => {
                        input = input.trim().to_string();
                        Ok(Some(Value::String(input)))
                    }
                    Err(e) => Err(format!("Error reading input: {}", e)),
                }
            }

            "read_file" => {
                if args.len() != 1 {
                    return Err(format!(
                        "read_file() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let filename = self.eval_expression(&args[0])?;
                if let Value::String(filename_str) = filename {
                    match fs::read_to_string(&filename_str) {
                        Ok(content) => Ok(Some(Value::String(content))),
                        Err(e) => Err(format!("Error reading file '{}': {}", filename_str, e)),
                    }
                } else {
                    Err("read_file() filename must be a string".to_string())
                }
            }

            "write_file" => {
                if args.len() != 2 {
                    return Err(format!(
                        "write_file() expects 2 arguments, got {}",
                        args.len()
                    ));
                }

                let filename = self.eval_expression(&args[0])?;
                let content = self.eval_expression(&args[1])?;

                if let Value::String(filename_str) = filename {
                    let content_str = match content {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };

                    let processed_content = content_str.replace("\\n", "\n");

                    match fs::write(&filename_str, processed_content) {
                        Ok(_) => Ok(Some(Value::Void)),
                        Err(e) => Err(format!("Error writing file '{}': {}", filename_str, e)),
                    }
                } else {
                    Err("write_file() filename must be a string".to_string())
                }
            }

            "append_file" => {
                if args.len() != 2 {
                    return Err(format!(
                        "append_file() expects 2 arguments, got {}",
                        args.len()
                    ));
                }

                let filename = self.eval_expression(&args[0])?;
                let content = self.eval_expression(&args[1])?;

                if let Value::String(filename_str) = filename {
                    let content_str = match content {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };

                    let processed_content = content_str.replace("\\n", "\n");

                    match fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&filename_str)
                    {
                        Ok(mut file) => {
                            use std::io::Write;
                            match file.write_all(processed_content.as_bytes()) {
                                Ok(_) => Ok(Some(Value::Void)),
                                Err(e) => Err(format!(
                                    "Error appending to file '{}': {}",
                                    filename_str, e
                                )),
                            }
                        }
                        Err(e) => Err(format!("Error opening file '{}': {}", filename_str, e)),
                    }
                } else {
                    Err("append_file() filename must be a string".to_string())
                }
            }

            "file_exists" => {
                if args.len() != 1 {
                    return Err(format!(
                        "file_exists() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let filename = self.eval_expression(&args[0])?;
                if let Value::String(filename_str) = filename {
                    let exists = Path::new(&filename_str).exists();
                    Ok(Some(Value::Bool(exists)))
                } else {
                    Err("file_exists() filename must be a string".to_string())
                }
            }

            "list_directory" => {
                if args.len() != 1 {
                    return Err(format!(
                        "list_directory() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    match fs::read_dir(&path_str) {
                        Ok(entries) => {
                            let names: Vec<Value> = entries
                                .filter_map(|e| e.ok())
                                .filter_map(|e| e.file_name().into_string().ok())
                                .map(Value::String)
                                .collect();
                            Ok(Some(Value::Array(names)))
                        }
                        Err(_) => Ok(Some(Value::Array(vec![]))),
                    }
                } else {
                    Err("list_directory() path must be a string".to_string())
                }
            }

            "create_directory" => {
                if args.len() != 1 {
                    return Err(format!(
                        "create_directory() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    match fs::create_dir_all(&path_str) {
                        Ok(_) => Ok(Some(Value::Bool(true))),
                        Err(_) => Ok(Some(Value::Bool(false))),
                    }
                } else {
                    Err("create_directory() path must be a string".to_string())
                }
            }

            "remove_file" => {
                if args.len() != 1 {
                    return Err(format!(
                        "remove_file() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    match fs::remove_file(&path_str) {
                        Ok(_) => Ok(Some(Value::Bool(true))),
                        Err(_) => Ok(Some(Value::Bool(false))),
                    }
                } else {
                    Err("remove_file() path must be a string".to_string())
                }
            }

            "remove_directory" => {
                if args.len() != 1 {
                    return Err(format!(
                        "remove_directory() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    match fs::remove_dir_all(&path_str) {
                        Ok(_) => Ok(Some(Value::Bool(true))),
                        Err(_) => Ok(Some(Value::Bool(false))),
                    }
                } else {
                    Err("remove_directory() path must be a string".to_string())
                }
            }

            "get_file_size" => {
                if args.len() != 1 {
                    return Err(format!(
                        "get_file_size() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    match fs::metadata(&path_str) {
                        Ok(meta) => Ok(Some(Value::Int(meta.len() as i64))),
                        Err(_) => Ok(Some(Value::Int(0))),
                    }
                } else {
                    Err("get_file_size() path must be a string".to_string())
                }
            }

            "is_dir" => {
                if args.len() != 1 {
                    return Err(format!("is_dir() expects 1 argument, got {}", args.len()));
                }

                let path_val = self.eval_expression(&args[0])?;
                if let Value::String(path_str) = path_val {
                    let is_dir = Path::new(&path_str).is_dir();
                    Ok(Some(Value::Bool(is_dir)))
                } else {
                    Err("is_dir() path must be a string".to_string())
                }
            }

            "sys_time" => {
                if !args.is_empty() {
                    return Err(format!(
                        "sys_time() expects 0 arguments, got {}",
                        args.len()
                    ));
                }

                let now = Local::now();
                Ok(Some(Value::String(now.format("%H:%M:%S").to_string())))
            }

            "sys_date" => {
                if !args.is_empty() {
                    return Err(format!(
                        "sys_date() expects 0 arguments, got {}",
                        args.len()
                    ));
                }

                let now = Local::now();
                Ok(Some(Value::String(now.format("%Y-%m-%d").to_string())))
            }

            "parse_int" => {
                if args.len() != 1 {
                    return Err(format!(
                        "parse_int() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let val = self.eval_expression(&args[0])?;
                if let Value::String(s) = val {
                    match s.parse::<i64>() {
                        Ok(n) => Ok(Some(Value::Int(n))),
                        Err(_) => Ok(Some(Value::Int(0))),
                    }
                } else {
                    Err("parse_int() expects a string argument".to_string())
                }
            }

            "char_code" => {
                if args.len() != 1 {
                    return Err(format!(
                        "char_code() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let val = self.eval_expression(&args[0])?;
                if let Value::String(s) = val {
                    let code = s.chars().next().map(|c| c as i64).unwrap_or(0);
                    Ok(Some(Value::Int(code)))
                } else {
                    Err("char_code() expects a string argument".to_string())
                }
            }

            "sys_timestamp" => {
                if !args.is_empty() {
                    return Err(format!(
                        "sys_timestamp() expects 0 arguments, got {}",
                        args.len()
                    ));
                }

                let now = Local::now();
                Ok(Some(Value::Float(
                    now.timestamp() as f64 + now.timestamp_subsec_millis() as f64 / 1000.0,
                )))
            }

            "format" => {
                if args.is_empty() {
                    return Err(format!(
                        "format() expects at least 1 argument, got {}",
                        args.len()
                    ));
                }

                let template = self.eval_expression(&args[0])?;
                if let Value::String(template_str) = template {
                    let mut result = template_str.clone();
                    let mut arg_index = 1;

                    while let Some(pos) = result.find("{}") {
                        if arg_index >= args.len() {
                            return Err(
                                "format() not enough arguments for placeholders".to_string()
                            );
                        }

                        let replacement_value = self.eval_expression(&args[arg_index])?;
                        let replacement = replacement_value.to_string();
                        result.replace_range(pos..pos + 2, &replacement);
                        arg_index += 1;
                    }

                    Ok(Some(Value::String(result)))
                } else {
                    Err("format() template must be a string".to_string())
                }
            }

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

            "tcp_listen" => {
                if args.len() != 2 {
                    return Err(format!(
                        "tcp_listen() expects 2 arguments, got {}",
                        args.len()
                    ));
                }
                let addr_val = self.eval_expression(&args[0])?;
                let _backlog_val = self.eval_expression(&args[1])?;
                let addr = match addr_val {
                    Value::String(s) => s,
                    _ => return Err("tcp_listen() address must be a string".to_string()),
                };
                let _ = _backlog_val;
                let listener =
                    TcpListener::bind(addr.as_str()).map_err(|e| format!("tcp_listen: {}", e))?;
                let id = self.alloc_tcp_id();
                self.tcp_listeners.insert(id, listener);
                Ok(Some(Value::TcpListener(id)))
            }

            "tcp_accept" => {
                if args.len() != 1 {
                    return Err(format!(
                        "tcp_accept() expects 1 argument, got {}",
                        args.len()
                    ));
                }
                let lid = match self.eval_expression(&args[0])? {
                    Value::TcpListener(id) => id,
                    _ => return Err("tcp_accept() requires a TcpListener".to_string()),
                };
                let listener = self
                    .tcp_listeners
                    .get_mut(&lid)
                    .ok_or_else(|| "tcp_accept: invalid TcpListener handle".to_string())?;
                let (stream, _addr) = listener
                    .accept()
                    .map_err(|e| format!("tcp_accept: {}", e))?;
                let sid = self.alloc_tcp_id();
                self.tcp_streams.insert(sid, stream);
                Ok(Some(Value::TcpStream(sid)))
            }

            "tcp_read" => {
                if args.len() != 2 {
                    return Err(format!(
                        "tcp_read() expects 2 arguments, got {}",
                        args.len()
                    ));
                }
                let sid = match self.eval_expression(&args[0])? {
                    Value::TcpStream(id) => id,
                    _ => return Err("tcp_read() requires a TcpStream".to_string()),
                };
                let max_bytes = match self.eval_expression(&args[1])? {
                    Value::Int(i) => i,
                    _ => return Err("tcp_read() max_bytes must be an int".to_string()),
                };
                if max_bytes <= 0 {
                    return Ok(Some(Value::String(String::new())));
                }
                let max_bytes = max_bytes as usize;
                let stream = self
                    .tcp_streams
                    .get_mut(&sid)
                    .ok_or_else(|| "tcp_read: invalid TcpStream handle".to_string())?;
                let mut buf = vec![0u8; max_bytes];
                let n = stream
                    .read(&mut buf)
                    .map_err(|e| format!("tcp_read: {}", e))?;
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                Ok(Some(Value::String(s)))
            }

            "tcp_write" => {
                if args.len() != 2 {
                    return Err(format!(
                        "tcp_write() expects 2 arguments, got {}",
                        args.len()
                    ));
                }
                let sid = match self.eval_expression(&args[0])? {
                    Value::TcpStream(id) => id,
                    _ => return Err("tcp_write() requires a TcpStream".to_string()),
                };
                let data = match self.eval_expression(&args[1])? {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                let stream = self
                    .tcp_streams
                    .get_mut(&sid)
                    .ok_or_else(|| "tcp_write: invalid TcpStream handle".to_string())?;
                let n = stream
                    .write(data.as_bytes())
                    .map_err(|e| format!("tcp_write: {}", e))?;
                stream
                    .flush()
                    .map_err(|e| format!("tcp_write: flush: {}", e))?;
                Ok(Some(Value::Int(n as i64)))
            }

            "tcp_close_stream" => {
                if args.len() != 1 {
                    return Err(format!(
                        "tcp_close_stream() expects 1 argument, got {}",
                        args.len()
                    ));
                }
                let sid = match self.eval_expression(&args[0])? {
                    Value::TcpStream(id) => id,
                    _ => return Err("tcp_close_stream() requires a TcpStream".to_string()),
                };
                if self.tcp_streams.remove(&sid).is_none() {
                    return Err("tcp_close_stream: invalid TcpStream handle".to_string());
                }
                Ok(Some(Value::Void))
            }

            "tcp_close_listener" => {
                if args.len() != 1 {
                    return Err(format!(
                        "tcp_close_listener() expects 1 argument, got {}",
                        args.len()
                    ));
                }
                let lid = match self.eval_expression(&args[0])? {
                    Value::TcpListener(id) => id,
                    _ => return Err("tcp_close_listener() requires a TcpListener".to_string()),
                };
                if self.tcp_listeners.remove(&lid).is_none() {
                    return Err("tcp_close_listener: invalid TcpListener handle".to_string());
                }
                Ok(Some(Value::Void))
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

            "dns_lookup" => {
                if args.len() != 1 {
                    return Err(format!(
                        "dns_lookup() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let host_val = self.eval_expression(&args[0])?;
                let host = match host_val {
                    Value::String(s) => s,
                    _ => return Err("dns_lookup() hostname must be a string".to_string()),
                };

                match (host.as_str(), 80u16).to_socket_addrs() {
                    Ok(mut iter) => {
                        let ip = iter.next().map(|a| a.ip().to_string()).unwrap_or_default();
                        Ok(Some(Value::String(ip)))
                    }
                    Err(_) => Ok(Some(Value::String(String::new()))),
                }
            }

            "reachable" => {
                if args.len() != 1 {
                    return Err(format!(
                        "reachable() expects 1 argument, got {}",
                        args.len()
                    ));
                }

                let host_val = self.eval_expression(&args[0])?;
                let host = match host_val {
                    Value::String(s) => s,
                    _ => return Err("reachable() hostname must be a string".to_string()),
                };

                let host = host.trim();
                if let Ok(addr) = host.parse::<SocketAddr>() {
                    return Ok(Some(Value::Bool(
                        TcpStream::connect_timeout(&addr, Duration::from_secs(4)).is_ok(),
                    )));
                }

                for port in [443u16, 80u16, 22u16] {
                    if let Ok(iter) = (host, port).to_socket_addrs() {
                        for addr in iter {
                            if TcpStream::connect_timeout(&addr, Duration::from_secs(4)).is_ok() {
                                return Ok(Some(Value::Bool(true)));
                            }
                        }
                    }
                }

                Ok(Some(Value::Bool(false)))
            }

            "enum_from_string" => {
                if args.len() != 2 {
                    return Err(format!(
                        "enum_from_string() expects 2 arguments, got {}",
                        args.len()
                    ));
                }

                let enum_name_value = self.eval_expression(&args[0])?;
                let variant_name_value = self.eval_expression(&args[1])?;

                if let (Value::String(enum_name), Value::String(variant_name)) =
                    (enum_name_value, variant_name_value)
                {
                    if let Some(variants) = self.enums.get(&enum_name) {
                        if variants.contains(&variant_name) {
                            Ok(Some(Value::Enum(enum_name, variant_name)))
                        } else {
                            Err(format!(
                                "Variant '{}' not found in enum '{}'. Available variants: {:?}",
                                variant_name, enum_name, variants
                            ))
                        }
                    } else {
                        Err(format!("Enum '{}' not found", enum_name))
                    }
                } else {
                    Err("enum_from_string() expects two string arguments".to_string())
                }
            }

            _ => Ok(None),
        }
    }
}
