//! The per-app MCP client: newline-delimited JSON-RPC 2.0 over a unix
//! domain socket. Sync `std::os::unix::net` throughout, matching the
//! crate's no-async style; read/write timeouts bound every operation so
//! one hung app cannot wedge the bus.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;

/// MCP protocol revision offered in `initialize`. The server's answer is
/// accepted without a match check — `tools/call` is stable across
/// revisions this client speaks.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Error)]
pub enum McpError {
    #[error("socket I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON on the wire: {0}")]
    Json(#[from] serde_json::Error),
    #[error("timed out waiting for the app")]
    Timeout,
    #[error("JSON-RPC error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("malformed response: {0}")]
    Protocol(String),
    #[error("tool failed: {0}")]
    Tool(String),
}

/// Map socket timeout errors to [`McpError::Timeout`].
fn map_io(e: std::io::Error) -> McpError {
    match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => McpError::Timeout,
        _ => McpError::Io(e),
    }
}

/// One connection to one app's MCP server. Dropped after the dispatch —
/// no session is kept alive across bus calls.
pub struct McpClient {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl McpClient {
    /// Connect to `path`, bounding every read/write by `timeout`. (The
    /// initial `connect(2)` itself has no std timeout hook; a dead
    /// listener still fails fast with `ECONNREFUSED`.)
    pub fn connect(path: &Path, timeout: Duration) -> Result<McpClient, McpError> {
        let stream = UnixStream::connect(path).map_err(McpError::Io)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(McpError::Io)?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(McpError::Io)?;
        let reader = BufReader::new(stream.try_clone().map_err(McpError::Io)?);
        Ok(McpClient {
            writer: stream,
            reader,
            next_id: 1,
        })
    }

    /// MCP handshake: `initialize` followed by the
    /// `notifications/initialized` notification. Returns the server's
    /// capabilities/server-info result, unvalidated by design.
    pub fn initialize(&mut self) -> Result<Value, McpError> {
        let result = self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "lisa-mcp-bus", "version": env!("CARGO_PKG_VERSION") },
            }),
        )?;
        self.notify("notifications/initialized", json!({}))?;
        Ok(result)
    }

    /// Invoke one tool and shape the MCP result into the plain `Value`
    /// the bus journals (see [`extract_tool_result`]).
    pub fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value, McpError> {
        let result = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )?;
        extract_tool_result(result)
    }

    /// Send a request and read messages until the response carrying our
    /// id arrives. Server-initiated notifications (and requests, which
    /// this minimal client never answers) are skipped.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        loop {
            let msg = self.recv()?;
            if msg.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = msg.get("error") {
                return Err(McpError::Rpc {
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(-32000),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string(),
                });
            }
            return msg.get("result").cloned().ok_or_else(|| {
                McpError::Protocol(format!("response to {method} has neither result nor error"))
            });
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), McpError> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn send(&mut self, msg: &Value) -> Result<(), McpError> {
        let mut line = serde_json::to_vec(msg)?;
        line.push(b'\n');
        self.writer.write_all(&line).map_err(map_io)?;
        self.writer.flush().map_err(map_io)
    }

    fn recv(&mut self) -> Result<Value, McpError> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).map_err(map_io)?;
        if n == 0 {
            return Err(McpError::Protocol("server closed the connection".into()));
        }
        Ok(serde_json::from_str(line.trim_end())?)
    }
}

/// Shape an MCP `tools/call` result into the bus's result `Value`:
/// `structuredContent` wins; a lone text block holding JSON is parsed
/// back into a value (a lone non-JSON text block becomes `Value::String`);
/// anything else is returned verbatim. `isError: true` maps to
/// [`McpError::Tool`] with the text content as the message.
fn extract_tool_result(result: Value) -> Result<Value, McpError> {
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(McpError::Tool(content_text(&result)));
    }
    if let Some(structured) = result.get("structuredContent") {
        return Ok(structured.clone());
    }
    if let Some(content) = result.get("content").and_then(Value::as_array)
        && content.len() == 1
        && content[0].get("type").and_then(Value::as_str) == Some("text")
        && let Some(text) = content[0].get("text").and_then(Value::as_str)
    {
        return Ok(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())));
    }
    Ok(result)
}

fn content_text(result: &Value) -> String {
    let texts: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|content| {
            content
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    if texts.is_empty() {
        "tool reported an error".to_string()
    } else {
        texts.join("; ")
    }
}
