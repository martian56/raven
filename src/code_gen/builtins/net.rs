use super::super::{Interpreter, Value};
use crate::ast::Expression;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

pub(super) fn call(
    interp: &mut Interpreter,
    name: &str,
    args: &[Expression],
) -> Result<Option<Value>, String> {
    match name {
        "tcp_listen" => {
            if args.len() != 2 {
                return Err(format!(
                    "tcp_listen() expects 2 arguments, got {}",
                    args.len()
                ));
            }
            let addr_val = interp.eval_expression(&args[0])?;
            let _backlog_val = interp.eval_expression(&args[1])?;
            let addr = match addr_val {
                Value::String(s) => s,
                _ => return Err("tcp_listen() address must be a string".to_string()),
            };
            let _ = _backlog_val;
            let listener =
                TcpListener::bind(addr.as_str()).map_err(|e| format!("tcp_listen: {}", e))?;
            let id = interp.alloc_tcp_id();
            interp.tcp_listeners.insert(id, listener);
            Ok(Some(Value::TcpListener(id)))
        }

        "tcp_accept" => {
            if args.len() != 1 {
                return Err(format!(
                    "tcp_accept() expects 1 argument, got {}",
                    args.len()
                ));
            }
            let lid = match interp.eval_expression(&args[0])? {
                Value::TcpListener(id) => id,
                _ => return Err("tcp_accept() requires a TcpListener".to_string()),
            };
            let listener = interp
                .tcp_listeners
                .get_mut(&lid)
                .ok_or_else(|| "tcp_accept: invalid TcpListener handle".to_string())?;
            let (stream, _addr) = listener
                .accept()
                .map_err(|e| format!("tcp_accept: {}", e))?;
            let sid = interp.alloc_tcp_id();
            interp.tcp_streams.insert(sid, stream);
            Ok(Some(Value::TcpStream(sid)))
        }

        "tcp_read" => {
            if args.len() != 2 {
                return Err(format!(
                    "tcp_read() expects 2 arguments, got {}",
                    args.len()
                ));
            }
            let sid = match interp.eval_expression(&args[0])? {
                Value::TcpStream(id) => id,
                _ => return Err("tcp_read() requires a TcpStream".to_string()),
            };
            let max_bytes = match interp.eval_expression(&args[1])? {
                Value::Int(i) => i,
                _ => return Err("tcp_read() max_bytes must be an int".to_string()),
            };
            if max_bytes <= 0 {
                return Ok(Some(Value::String(String::new())));
            }
            let max_bytes = max_bytes as usize;
            let stream = interp
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
            let sid = match interp.eval_expression(&args[0])? {
                Value::TcpStream(id) => id,
                _ => return Err("tcp_write() requires a TcpStream".to_string()),
            };
            let data = match interp.eval_expression(&args[1])? {
                Value::String(s) => s,
                other => other.to_string(),
            };
            let stream = interp
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
            let sid = match interp.eval_expression(&args[0])? {
                Value::TcpStream(id) => id,
                _ => return Err("tcp_close_stream() requires a TcpStream".to_string()),
            };
            if interp.tcp_streams.remove(&sid).is_none() {
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
            let lid = match interp.eval_expression(&args[0])? {
                Value::TcpListener(id) => id,
                _ => return Err("tcp_close_listener() requires a TcpListener".to_string()),
            };
            if interp.tcp_listeners.remove(&lid).is_none() {
                return Err("tcp_close_listener: invalid TcpListener handle".to_string());
            }
            Ok(Some(Value::Void))
        }

        "dns_lookup" => {
            if args.len() != 1 {
                return Err(format!(
                    "dns_lookup() expects 1 argument, got {}",
                    args.len()
                ));
            }

            let host_val = interp.eval_expression(&args[0])?;
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

            let host_val = interp.eval_expression(&args[0])?;
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

        _ => Ok(None),
    }
}
