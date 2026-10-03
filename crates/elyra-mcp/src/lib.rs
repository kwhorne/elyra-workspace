//! A small Model Context Protocol server: JSON-RPC over HTTP POST on
//! 127.0.0.1 with bearer-token auth (the "Streamable HTTP" transport without
//! server-sent events), plus a stdio bridge for clients that only launch
//! local commands.
//!
//! The server knows nothing about Elyra; a [`Handler`] supplies tools and
//! runs calls (blocking, on a connection thread).

use anyhow::{Context as _, Result, anyhow};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
const MAX_BODY: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Who is calling, as decided by [`Handler::authorize`].
pub type Principal = String;

pub trait Handler: Send + Sync + 'static {
    /// Map a bearer token to a caller, or refuse it.
    fn authorize(&self, token: &str) -> Option<Principal>;
    fn tools(&self, principal: &Principal) -> Vec<Tool>;
    /// Run a tool. `Ok` text is the result; `Err` text is reported to the
    /// model as a tool error.
    fn call(&self, principal: &Principal, name: &str, arguments: Value) -> Result<String, String>;
}

pub struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
}

impl Server {
    /// Listen on 127.0.0.1:`port` (0 picks a free port).
    pub fn start(port: u16, handler: Arc<dyn Handler>) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
            .context("starting the MCP server")?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::Builder::new()
            .name("mcp-server".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if flag.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let handler = handler.clone();
                    let _ = std::thread::Builder::new()
                        .name("mcp-conn".into())
                        .spawn(move || {
                            if let Err(err) = serve(stream, handler.as_ref()) {
                                log::debug!("mcp connection: {err:#}");
                            }
                        });
                }
            })?;
        Ok(Self { addr, stop })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.addr.port())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop.
        let _ = TcpStream::connect(self.addr);
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn read_request(reader: &mut impl BufRead) -> Result<Request> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| anyhow!("empty request"))?
        .to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((key, value)) = header.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    let length: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    anyhow::ensure!(length <= MAX_BODY, "request too large");
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) -> Result<()> {
    let content_type = if body.is_empty() {
        "text/plain"
    } else {
        "application/json"
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    Ok(())
}

fn serve(mut stream: TcpStream, handler: &dyn Handler) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let request = read_request(&mut BufReader::new(stream.try_clone()?))?;
    if request.path.split('?').next() != Some("/mcp") {
        return respond(&mut stream, "404 Not Found", "");
    }
    if request.method != "POST" {
        // No server-initiated stream.
        return respond(&mut stream, "405 Method Not Allowed", "");
    }
    let token = request
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let Some(principal) = handler.authorize(token) else {
        return respond(&mut stream, "401 Unauthorized", "");
    };
    let message: Value = match serde_json::from_slice(&request.body) {
        Ok(message) => message,
        Err(err) => {
            let body = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": err.to_string() } });
            return respond(&mut stream, "400 Bad Request", &body.to_string());
        }
    };
    let response = match &message {
        Value::Array(batch) => {
            let replies: Vec<Value> = batch
                .iter()
                .filter_map(|m| handle(handler, &principal, m))
                .collect();
            (!replies.is_empty()).then(|| Value::Array(replies))
        }
        single => handle(handler, &principal, single),
    };
    match response {
        Some(body) => respond(&mut stream, "200 OK", &body.to_string()),
        None => respond(&mut stream, "202 Accepted", ""),
    }
}

/// Handle one JSON-RPC message; notifications get no reply.
pub fn handle(handler: &dyn Handler, principal: &Principal, message: &Value) -> Option<Value> {
    let id = message.get("id").filter(|id| !id.is_null())?.clone();
    let params = &message["params"];
    let result = match message["method"].as_str().unwrap_or("") {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "elyra-workspace", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Elyra Workspace: list, read, create and steer coding-agent threads."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": handler
                .tools(principal)
                .into_iter()
                .map(|tool| json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                }))
                .collect::<Vec<_>>()
        })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            let known = handler.tools(principal).iter().any(|t| t.name == name);
            if known {
                let (text, is_error) = match handler.call(principal, name, arguments) {
                    Ok(text) => (text, false),
                    Err(text) => (text, true),
                };
                Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }))
            } else {
                Err((-32602, format!("unknown tool: {name}")))
            }
        }
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    })
}

/// POST one JSON-RPC message; `None` for 202 (notification accepted).
pub fn post(url: &str, token: &str, body: &str) -> Result<Option<String>> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow!("only http:// URLs are supported"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let mut stream = TcpStream::connect(host).with_context(|| format!("connecting to {host}"))?;
    write!(
        stream,
        "POST /{path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status)?;
    let code: u16 = status
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let mut length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((key, value)) = header.split_once(':')
            && key.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = Vec::new();
    match length {
        Some(length) => {
            body.resize(length, 0);
            reader.read_exact(&mut body)?;
        }
        None => {
            reader.read_to_end(&mut body)?;
        }
    }
    match code {
        200 => Ok(Some(String::from_utf8_lossy(&body).into_owned())),
        202 => Ok(None),
        401 => Err(anyhow!(
            "Elyra Workspace refused the token (unpaired client?)"
        )),
        _ => Err(anyhow!("unexpected HTTP status {code}")),
    }
}

/// Relay newline-delimited JSON-RPC between stdio and the HTTP server, for
/// MCP clients that launch a local command (Claude Desktop, Codex).
pub fn run_bridge(url: &str, token: &str) -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match post(url, token, &line) {
            Ok(reply) => reply,
            Err(err) => {
                // Answer requests with an error so the client doesn't hang.
                let id = serde_json::from_str::<Value>(&line)
                    .ok()
                    .and_then(|m| m.get("id").cloned())
                    .filter(|id| !id.is_null());
                id.map(|id| {
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32000, "message": format!("Elyra Workspace is not reachable: {err:#}") }
                    })
                    .to_string()
                })
            }
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{}", reply.replace('\n', ""))?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl Handler for Echo {
        fn authorize(&self, token: &str) -> Option<Principal> {
            (token == "secret").then(|| "tester".to_string())
        }

        fn tools(&self, _: &Principal) -> Vec<Tool> {
            vec![Tool {
                name: "echo".into(),
                description: "Echo the text".into(),
                input_schema: json!({ "type": "object", "properties": { "text": { "type": "string" } } }),
            }]
        }

        fn call(&self, principal: &Principal, _: &str, arguments: Value) -> Result<String, String> {
            match arguments["text"].as_str() {
                Some("fail") => Err("failed on purpose".into()),
                Some(text) => Ok(format!("{principal}: {text}")),
                None => Err("missing text".into()),
            }
        }
    }

    fn call(url: &str, token: &str, message: Value) -> Result<Option<Value>> {
        Ok(
            post(url, token, &message.to_string())?
                .map(|body| serde_json::from_str(&body).unwrap()),
        )
    }

    #[test]
    fn serves_tools_over_http() {
        let server = Server::start(0, Arc::new(Echo)).unwrap();
        let url = server.url();
        assert!(
            call(
                &url,
                "wrong",
                json!({"jsonrpc":"2.0","id":1,"method":"ping"})
            )
            .is_err()
        );

        let init = call(&url, "secret", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}))
            .unwrap()
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(
            call(
                &url,
                "secret",
                json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            )
            .unwrap(),
            None,
            "notifications are accepted without a body"
        );
        let tools = call(
            &url,
            "secret",
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(tools["result"]["tools"][0]["name"], "echo");
        let result = call(&url, "secret", json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}))
            .unwrap()
            .unwrap();
        assert_eq!(result["result"]["content"][0]["text"], "tester: hi");
        assert_eq!(result["result"]["isError"], false);
        let failed = call(&url, "secret", json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"echo","arguments":{"text":"fail"}}}))
            .unwrap()
            .unwrap();
        assert_eq!(failed["result"]["isError"], true);
        let unknown = call(
            &url,
            "secret",
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nope"}}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
    }
}
