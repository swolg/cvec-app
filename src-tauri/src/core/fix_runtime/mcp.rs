//! A private, stateless Streamable HTTP MCP endpoint for one authorized turn.
//! No discovery, browser access, persistent credentials or arbitrary routing.
use super::*;
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(super) struct Bridge {
    pub url: String,
    pub authorization: String,
    stopped: Arc<AtomicBool>,
}
impl Bridge {
    pub fn start(
        tools: Value,
        sink: Arc<dyn FixSink>,
        session: String,
        turn: String,
    ) -> Result<Self, Error> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let authorization = format!("Bearer {}{}", identity()?, identity()?);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let auth = authorization.clone();
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30 * 60);
            while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        let response = request(&mut stream, &address.to_string(), &auth, &stop)
                            .map(|value| dispatch(value, &tools, &*sink, &session, &turn));
                        match response {
                            Ok(Some(value)) => {
                                let _ = respond(&mut stream, 200, &value.to_string());
                            }
                            Ok(None) => {
                                let _ = respond(&mut stream, 202, "");
                            }
                            Err(()) => {
                                let _ = respond(&mut stream, 403, "");
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            url: format!("http://{address}/mcp"),
            authorization,
            stopped,
        })
    }
    pub fn revoke(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.revoke();
    }
}

fn request(
    stream: &mut TcpStream,
    host: &str,
    auth: &str,
    stopped: &AtomicBool,
) -> Result<Value, ()> {
    let mut reader = BufReader::new(RequestReader {
        stream,
        stopped,
        deadline: Instant::now() + Duration::from_secs(2),
    });
    let mut headers = Vec::new();
    loop {
        let mut line = Vec::new();
        reader
            .by_ref()
            .take(8193)
            .read_until(b'\n', &mut line)
            .map_err(|_| ())?;
        if line.is_empty() || line.len() > 8192 {
            return Err(());
        }
        headers.extend_from_slice(&line);
        if headers.len() > 16384 {
            return Err(());
        }
        if line == b"\r\n" {
            break;
        }
    }
    let header = std::str::from_utf8(&headers).map_err(|_| ())?;
    let mut lines = header.split("\r\n");
    if lines.next() != Some("POST /mcp HTTP/1.1") {
        return Err(());
    }
    let mut fields = std::collections::HashMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(())?;
        if fields
            .insert(name.to_ascii_lowercase(), value.trim())
            .is_some()
        {
            return Err(());
        }
    }
    if fields.get("host") != Some(&host)
        || !fields
            .get("authorization")
            .is_some_and(|value| constant_time_eq(value.as_bytes(), auth.as_bytes()))
        || fields.contains_key("origin")
        || fields.contains_key("transfer-encoding")
        || !fields.get("content-type").is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
        })
    {
        return Err(());
    }
    let length: usize = fields
        .get("content-length")
        .ok_or(())?
        .parse()
        .map_err(|_| ())?;
    if length == 0 || length > 1024 * 1024 || stopped.load(Ordering::SeqCst) {
        return Err(());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|_| ())?;
    if stopped.load(Ordering::SeqCst) {
        return Err(());
    }
    serde_json::from_slice(&body).map_err(|_| ())
}

// A timeout on each socket read alone lets a slow sender keep the single
// worker occupied indefinitely. Bound the entire header/body read instead.
struct RequestReader<'a> {
    stream: &'a mut TcpStream,
    stopped: &'a AtomicBool,
    deadline: Instant,
}
impl Read for RequestReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || self.stopped.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "MCP request expired",
            ));
        }
        self.stream.set_read_timeout(Some(remaining))?;
        self.stream.read(buffer)
    }
}
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&a, &b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}", if status == 200 { "OK" } else if status == 202 { "Accepted" } else { "Forbidden" }, body.len())?;
    stream.flush()
}
fn dispatch(
    value: Value,
    tools: &Value,
    sink: &dyn FixSink,
    session: &str,
    turn: &str,
) -> Option<Value> {
    if value["jsonrpc"] != "2.0"
        || !value["method"].is_string()
        || value
            .get("id")
            .is_some_and(|id| !id.is_string() && !id.is_number())
        || value
            .get("params")
            .is_some_and(|params| !params.is_object())
    {
        return Some(
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid request"}}),
        );
    }
    let id = value.get("id")?.clone();
    let result = match value.get("method").and_then(Value::as_str) {
        Some("initialize") => {
            json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"cvec-workspace","version":"1"}})
        }
        Some("ping") => json!({}),
        Some("tools/list") => {
            json!({"tools":tools.as_array().into_iter().flatten().map(|tool|json!({"name":tool["name"],"description":tool["description"],"inputSchema":tool["inputSchema"]})).collect::<Vec<_>>()})
        }
        Some("tools/call") => {
            let name = value
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = value
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            if name.is_empty() || !args.is_object() {
                return Some(
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"Invalid tool arguments"}}),
                );
            }
            let known = tools
                .as_array()
                .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == name));
            let result = if known {
                sink.call(name, &args, session, turn)
            } else {
                Err(Error::App("Unknown workspace tool.".into()))
            };
            match result {
                Ok(value) => {
                    json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
                }
                Err(error) => {
                    json!({"content":[{"type":"text","text":error.to_string()}],"isError":true})
                }
            }
        }
        _ => {
            return Some(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method unavailable"}}),
            )
        }
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct Sink(AtomicUsize);
    impl FixSink for Sink {
        fn call(&self, _: &str, _: &Value, session: &str, turn: &str) -> Result<Value, Error> {
            assert_eq!((session, turn), ("session", "turn"));
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"ok":true}))
        }
        fn event(&self, _: FixEvent) {}
    }

    #[test]
    fn invalid_rpc_and_tool_notifications_cannot_execute() {
        let sink = Sink::default();
        let tools = json!([{"name":"edit","inputSchema":{"type":"object"}}]);
        for value in [
            json!({"id":1,"method":"tools/call","params":{"name":"edit"}}),
            json!({"jsonrpc":"2.0","id":false,"method":"tools/call","params":{"name":"edit"}}),
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"edit","arguments":[]}}),
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"shell"}}),
        ] {
            let result = dispatch(value, &tools, &sink, "session", "turn").unwrap();
            assert!(result.get("error").is_some() || result["result"]["isError"] == true);
        }
        assert!(dispatch(
            json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"edit"}}),
            &tools,
            &sink,
            "session",
            "turn"
        )
        .is_none());
        assert_eq!(sink.0.load(Ordering::SeqCst), 0);
        let result = dispatch(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"edit","arguments":{}}}), &tools, &sink, "session", "turn").unwrap();
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(sink.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn socket_reads_obey_the_total_deadline_and_revocation() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        client.write_all(b"ok").unwrap();
        let stopped = AtomicBool::new(false);
        let mut reader = RequestReader {
            stream: &mut server,
            stopped: &stopped,
            deadline: Instant::now() - Duration::from_millis(1),
        };
        assert_eq!(
            reader.read(&mut [0]).unwrap_err().kind(),
            std::io::ErrorKind::TimedOut
        );
        reader.deadline = Instant::now() + Duration::from_secs(1);
        stopped.store(true, Ordering::SeqCst);
        assert_eq!(
            reader.read(&mut [0]).unwrap_err().kind(),
            std::io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn accepts_case_insensitive_json_content_type_and_constant_time_auth() {
        assert!(constant_time_eq(b"Bearer token", b"Bearer token"));
        assert!(!constant_time_eq(b"Bearer token", b"Bearer tokeX"));
        assert!(!constant_time_eq(b"short", b"longer"));

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let auth = "Bearer fixture-auth-value";
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let payload = format!(
            "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nAuthorization: {auth}\r\nContent-Type: Application/JSON ; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let writer = std::thread::spawn(move || {
            let mut client = TcpStream::connect(addr).unwrap();
            client.write_all(payload.as_bytes()).unwrap();
        });
        let (mut server, _) = listener.accept().unwrap();
        let stopped = AtomicBool::new(false);
        let parsed = request(&mut server, &addr.to_string(), auth, &stopped).unwrap();
        writer.join().unwrap();
        assert_eq!(parsed["method"], "ping");
    }
}
