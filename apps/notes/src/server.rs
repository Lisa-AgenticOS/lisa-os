//! The MCP server: newline-delimited JSON-RPC 2.0 over a unix socket —
//! exactly the wire `mcp_bus::McpClient` speaks (initialize,
//! notifications/initialized, tools/call). Requests get a response with
//! the matching id; notifications are swallowed; tool failures are MCP
//! `isError: true` *results* (not JSON-RPC errors), per the contract in
//! `libs/mcp-bus/src/client.rs::extract_tool_result`.

use crate::storage::Store;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};

/// MCP protocol revision this server speaks (matches mcp-bus).
pub const PROTOCOL_VERSION: &str = "2024-11-05";

const MAX_TITLE_CHARS: usize = 120;
const MAX_BODY_CHARS: usize = 4000;

/// Accept connections one at a time, forever. The bus opens a fresh
/// connection per dispatch and drops it after the call, so sequential
/// accept is all the concurrency this server needs.
pub fn serve(listener: UnixListener, store: &Store) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_conn(stream, store) {
                    eprintln!("lisa-notes: connection dropped: {e}");
                }
            }
            Err(e) => eprintln!("lisa-notes: accept failed: {e}"),
        }
    }
}

fn handle_conn(stream: UnixStream, store: &Store) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(()); // client hung up
        }
        let Ok(msg) = serde_json::from_str::<Value>(line.trim_end()) else {
            return Ok(()); // garbage: hang up so the dispatch fails fast, not by timeout
        };
        let Some(id) = msg.get("id").cloned() else {
            continue; // notification (notifications/initialized & co.): no response
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let response = match method {
            "initialize" => initialize(id, &msg),
            "tools/call" => tools_call(id, &msg, store),
            other => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("no such method {other}") },
            }),
        };
        writer.write_all(response.to_string().as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
}

fn initialize(id: Value, msg: &Value) -> Value {
    // Answer with the offered revision when there is one; the client
    // accepts without a match check.
    let offered = msg
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(PROTOCOL_VERSION);
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": offered,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "lisa-notes", "version": env!("CARGO_PKG_VERSION") },
        },
    })
}

fn tools_call(id: Value, msg: &Value, store: &Store) -> Value {
    let name = msg
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = msg
        .pointer("/params/arguments")
        .cloned()
        .unwrap_or(json!({}));
    let result = match name {
        "create_note" => create_note(store, &args),
        "list_notes" => list_notes(store),
        "delete_note" => delete_note(store, &args),
        "restore_note" => restore_note(store, &args),
        other => tool_error(format!("unknown tool: {other}")),
    };
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn create_note(store: &Store, args: &Value) -> Value {
    let title = match required_str(args, "title", "create_note") {
        Ok(t) => t,
        Err(e) => return e,
    };
    if title.chars().count() > MAX_TITLE_CHARS {
        return tool_error(format!(
            "create_note: \"title\" exceeds {MAX_TITLE_CHARS} characters"
        ));
    }
    let body = match args.get("body") {
        None | Some(Value::Null) => "",
        Some(v) => match v.as_str() {
            Some(s) => s,
            None => return tool_error("create_note: \"body\" must be a string"),
        },
    };
    if body.chars().count() > MAX_BODY_CHARS {
        return tool_error(format!(
            "create_note: \"body\" exceeds {MAX_BODY_CHARS} characters"
        ));
    }
    match store.create(title, body) {
        Ok(id) => structured(json!({ "id": id, "title": title })),
        Err(e) => tool_error(format!("create_note: {e}")),
    }
}

fn list_notes(store: &Store) -> Value {
    match store.list() {
        Ok(notes) => structured(json!({
            "notes": notes
                .iter()
                .map(|n| json!({ "id": n.id, "title": n.title, "created": n.created }))
                .collect::<Vec<_>>(),
        })),
        Err(e) => tool_error(format!("list_notes: {e}")),
    }
}

fn delete_note(store: &Store, args: &Value) -> Value {
    let id = match required_id(args, "delete_note") {
        Ok(id) => id,
        Err(e) => return e,
    };
    match store.delete(id) {
        Ok(true) => structured(json!({ "id": id, "restored": false })),
        Ok(false) => tool_error(format!("delete_note: no active note with id {id}")),
        Err(e) => tool_error(format!("delete_note: {e}")),
    }
}

fn restore_note(store: &Store, args: &Value) -> Value {
    let id = match required_id(args, "restore_note") {
        Ok(id) => id,
        Err(e) => return e,
    };
    match store.restore(id) {
        Ok(true) => structured(json!({ "id": id, "restored": true })),
        Ok(false) => tool_error(format!("restore_note: no deleted note with id {id}")),
        Err(e) => tool_error(format!("restore_note: {e}")),
    }
}

fn required_str<'a>(args: &'a Value, field: &str, tool: &str) -> Result<&'a str, Value> {
    match args.get(field) {
        None => Err(tool_error(format!(
            "{tool}: missing required argument \"{field}\""
        ))),
        Some(v) => v
            .as_str()
            .ok_or_else(|| tool_error(format!("{tool}: \"{field}\" must be a string"))),
    }
}

fn required_id(args: &Value, tool: &str) -> Result<i64, Value> {
    match args.get("id") {
        None => Err(tool_error(format!(
            "{tool}: missing required argument \"id\""
        ))),
        Some(v) => v
            .as_i64()
            .ok_or_else(|| tool_error(format!("{tool}: \"id\" must be an integer"))),
    }
}

/// Success shaped so the bus's `extract_tool_result` reads it directly.
fn structured(v: Value) -> Value {
    json!({ "structuredContent": v })
}

/// Tool failure as an MCP error result — the bus maps this to
/// `McpError::Tool` with the text as the message.
fn tool_error(msg: impl Into<String>) -> Value {
    json!({
        "isError": true,
        "content": [{ "type": "text", "text": msg.into() }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_bus::{McpClient, McpError};
    use std::thread;
    use std::time::Duration;

    /// The full agentd-side stack against this app: real socket, real
    /// `McpClient`, real SQLite file — same frames `McpDispatcher` sends.
    #[test]
    fn end_to_end_over_a_real_socket() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("app.lisaos.notes.sock");
        let store = Store::open(&dir.path().join("notes.db")).unwrap();
        let listener = UnixListener::bind(&sock).unwrap();
        let _server = thread::spawn(move || serve(listener, &store));

        let mut client = McpClient::connect(&sock, Duration::from_secs(5)).unwrap();
        let info = client.initialize().unwrap();
        assert_eq!(info["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(info["serverInfo"]["name"], "lisa-notes");

        // create
        let created = client
            .call_tool(
                "create_note",
                &json!({"title": "first", "body": "hello bus"}),
            )
            .unwrap();
        let id = created["id"].as_i64().unwrap();
        assert_eq!(created["title"], "first");

        // list
        let list = client.call_tool("list_notes", &json!({})).unwrap();
        let notes = list["notes"].as_array().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0]["id"], id);
        assert_eq!(notes[0]["title"], "first");
        assert!(notes[0]["created"].as_str().unwrap().ends_with('Z'));

        // delete → hidden; a second delete is a tool error
        let deleted = client.call_tool("delete_note", &json!({"id": id})).unwrap();
        assert_eq!(deleted, json!({"id": id, "restored": false}));
        assert_eq!(
            client.call_tool("list_notes", &json!({})).unwrap(),
            json!({"notes": []})
        );
        let err = client
            .call_tool("delete_note", &json!({"id": id}))
            .unwrap_err();
        assert!(
            matches!(err, McpError::Tool(ref msg) if msg.contains("no active note")),
            "{err:?}"
        );

        // restore → back; a second restore is a tool error
        let restored = client
            .call_tool("restore_note", &json!({"id": id}))
            .unwrap();
        assert_eq!(restored, json!({"id": id, "restored": true}));
        let err = client
            .call_tool("restore_note", &json!({"id": id}))
            .unwrap_err();
        assert!(
            matches!(err, McpError::Tool(ref msg) if msg.contains("no deleted note")),
            "{err:?}"
        );
        assert_eq!(
            client.call_tool("list_notes", &json!({})).unwrap()["notes"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        // validation happens at the app too, not only at the bus
        let err = client
            .call_tool("create_note", &json!({"body": "no title"}))
            .unwrap_err();
        assert!(
            matches!(err, McpError::Tool(ref msg) if msg.contains("missing required argument")),
            "{err:?}"
        );
        let long = "x".repeat(MAX_TITLE_CHARS + 1);
        let err = client
            .call_tool("create_note", &json!({"title": long}))
            .unwrap_err();
        assert!(
            matches!(err, McpError::Tool(ref msg) if msg.contains("exceeds")),
            "{err:?}"
        );

        // unknown tool → isError, surfaced as McpError::Tool
        let err = client.call_tool("burn_note", &json!({})).unwrap_err();
        assert!(
            matches!(err, McpError::Tool(ref msg) if msg.contains("unknown tool: burn_note")),
            "{err:?}"
        );

        // the bus drops the connection after every dispatch; the next
        // one must get a fresh handshake on a fresh connection
        drop(client);
        let mut again = McpClient::connect(&sock, Duration::from_secs(5)).unwrap();
        again.initialize().unwrap();
        let list = again.call_tool("list_notes", &json!({})).unwrap();
        assert_eq!(list["notes"].as_array().unwrap().len(), 1);
    }
}
