//! The bus-facing dispatcher: resolves an app's socket and runs one
//! `tools/call` per `dispatch()`. The [`Dispatcher`] trait mirrors
//! agentd's `bus::Dispatcher` exactly (ADR-0009 names it the seam this
//! crate slots into) so `McpDispatcher` swaps in for `NullDispatcher`
//! without touching the bus state machine.

use crate::DEFAULT_SOCKET_DIR;
use crate::client::McpClient;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

/// How long any single socket operation may block before the dispatch
/// fails — one hung app must not wedge the bus.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// A failed dispatch, typed so the envelope's provenance tag survives
/// the error door (#313). A plain `String` here was how a `provenance`
/// an app put on its error reply vanished before any caller saw it —
/// and an error message quotes exactly the attacker-chosen content
/// (filenames, subject lines, page titles) the tag exists to mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchFailure {
    pub error: String,
    /// The app edge's own claim from the reply envelope; `None` for
    /// transport failures, where no app spoke at all.
    pub provenance: Option<String>,
}

impl DispatchFailure {
    /// A failure with no app behind it (connect, handshake, transport).
    pub fn transport(error: impl Into<String>) -> DispatchFailure {
        DispatchFailure {
            error: error.into(),
            provenance: None,
        }
    }
}

impl std::fmt::Display for DispatchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error)
    }
}

/// The transport seam the Agent Bus dispatches through. Same signature
/// as agentd's `bus::Dispatcher`; kept crate-local so `mcp-bus` does not
/// depend on the daemon.
pub trait Dispatcher: Send + Sync {
    fn dispatch(&self, app_id: &str, tool: &str, args: &Value) -> Result<Value, DispatchFailure>;
}

/// Per-app unix-socket MCP dispatcher: connects to
/// `<base_dir>/<app_id>.sock`, handshakes, and calls the tool, all
/// within one short-lived connection per dispatch. Socket activation
/// (`mcp.activatable`) is not implemented here — the app's socket must
/// already be live; otherwise the dispatch fails cleanly (and the bus
/// ledgers it as failed, exactly like `NullDispatcher` did).
pub struct McpDispatcher {
    base_dir: PathBuf,
    timeout: Duration,
}

impl McpDispatcher {
    pub fn new(base_dir: impl Into<PathBuf>) -> McpDispatcher {
        McpDispatcher {
            base_dir: base_dir.into(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> McpDispatcher {
        self.timeout = timeout;
        self
    }

    /// Socket path for one app: `<base_dir>/<app_id>.sock`.
    pub fn socket_path(&self, app_id: &str) -> PathBuf {
        self.base_dir.join(format!("{app_id}.sock"))
    }
}

impl Default for McpDispatcher {
    fn default() -> McpDispatcher {
        McpDispatcher::new(DEFAULT_SOCKET_DIR)
    }
}

impl Dispatcher for McpDispatcher {
    fn dispatch(&self, app_id: &str, tool: &str, args: &Value) -> Result<Value, DispatchFailure> {
        let path = self.socket_path(app_id);
        let mut client = McpClient::connect(&path, self.timeout).map_err(|e| {
            DispatchFailure::transport(format!("{app_id}: connect {}: {e}", path.display()))
        })?;
        client
            .initialize()
            .map_err(|e| DispatchFailure::transport(format!("{app_id}: initialize: {e}")))?;
        client.call_tool(tool, args).map_err(|e| match e {
            // The one error an APP produced: its envelope tag rides
            // along, so the consumer can taint on a failed call whose
            // message quotes untrusted content (#313).
            crate::McpError::Tool { ref provenance, .. } => DispatchFailure {
                error: format!("{app_id}/{tool}: {e}"),
                provenance: provenance.clone(),
            },
            other => DispatchFailure::transport(format!("{app_id}/{tool}: {other}")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// A tiny in-process MCP server speaking the wire protocol the
    /// client expects: answer `initialize`, swallow notifications, hand
    /// `tools/call` to `on_call` (Ok → result, Err → JSON-RPC error).
    /// Records request methods so tests can assert the handshake order.
    fn serve(
        listener: UnixListener,
        methods: Arc<Mutex<Vec<String>>>,
        on_call: impl Fn(&str, Value) -> Result<Value, Value> + Send + 'static,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break; // client hung up
                }
                let msg: Value = serde_json::from_str(line.trim_end()).unwrap();
                let Some(id) = msg.get("id").cloned() else {
                    continue; // notification: no response
                };
                let method = msg["method"].as_str().unwrap().to_string();
                methods.lock().unwrap().push(method.clone());
                let response = match method.as_str() {
                    "initialize" => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "protocolVersion": msg["params"]["protocolVersion"].clone(),
                            "capabilities": { "tools": {} },
                            "serverInfo": { "name": "mock-app", "version": "0" },
                        }
                    }),
                    "tools/call" => match on_call(
                        msg["params"]["name"].as_str().unwrap(),
                        msg["params"]["arguments"].clone(),
                    ) {
                        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
                    },
                    other => json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": { "code": -32601, "message": format!("no such method {other}") }
                    }),
                };
                writer
                    .write_all(serde_json::to_string(&response).unwrap().as_bytes())
                    .unwrap();
                writer.write_all(b"\n").unwrap();
                writer.flush().unwrap();
            }
        })
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        dispatcher: McpDispatcher,
        listener: UnixListener,
        methods: Arc<Mutex<Vec<String>>>,
    }

    fn fixture(app_id: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(dir.path().join(format!("{app_id}.sock"))).unwrap();
        Fixture {
            dispatcher: McpDispatcher::new(dir.path()),
            listener,
            _dir: dir,
            methods: Arc::new(Mutex::new(Vec::new())),
        }
    }

    const APP: &str = "org.gnome.Calendar";

    #[test]
    fn dispatch_handshakes_then_returns_structured_content() {
        let f = fixture(APP);
        let server = serve(f.listener, Arc::clone(&f.methods), |name, args| {
            assert_eq!(name, "add_event");
            assert_eq!(args, json!({"title": "dentist"}));
            Ok(json!({
                "content": [{ "type": "text", "text": "{\"event_id\": \"evt-1\"}" }],
                "structuredContent": { "event_id": "evt-1" },
            }))
        });
        let result = f
            .dispatcher
            .dispatch(APP, "add_event", &json!({"title": "dentist"}))
            .unwrap();
        assert_eq!(result, json!({ "event_id": "evt-1" }));
        server.join().unwrap();
        assert_eq!(
            *f.methods.lock().unwrap(),
            vec!["initialize".to_string(), "tools/call".to_string()],
            "handshake must precede the call"
        );
    }

    #[test]
    fn dispatch_parses_a_lone_json_text_block() {
        let f = fixture(APP);
        let server = serve(f.listener, Arc::clone(&f.methods), |_, _| {
            Ok(json!({
                "content": [{ "type": "text", "text": "[\"a\", \"b\"]" }],
            }))
        });
        let result = f
            .dispatcher
            .dispatch(APP, "list_events", &json!({}))
            .unwrap();
        assert_eq!(result, json!(["a", "b"]));
        server.join().unwrap();
    }

    #[test]
    fn dispatch_maps_jsonrpc_error_to_err() {
        let f = fixture(APP);
        let server = serve(f.listener, Arc::clone(&f.methods), |_, _| {
            Err(json!({ "code": -32602, "message": "bad args" }))
        });
        let err = f
            .dispatcher
            .dispatch(APP, "add_event", &json!({}))
            .unwrap_err();
        assert!(err.error.contains("bad args"), "{err}");
        assert!(err.error.contains("-32602"), "{err}");
        assert!(err.error.contains(APP), "{err}");
        server.join().unwrap();
    }

    #[test]
    fn dispatch_maps_tool_error_result_to_err() {
        let f = fixture(APP);
        let server = serve(f.listener, Arc::clone(&f.methods), |_, _| {
            Ok(json!({
                "isError": true,
                "content": [{ "type": "text", "text": "calendar is read-only" }],
            }))
        });
        let err = f
            .dispatcher
            .dispatch(APP, "add_event", &json!({}))
            .unwrap_err();
        assert!(err.error.contains("calendar is read-only"), "{err}");
        server.join().unwrap();
    }

    #[test]
    fn missing_socket_is_a_clean_error() {
        let dir = tempfile::tempdir().unwrap();
        let dispatcher = McpDispatcher::new(dir.path());
        let err = dispatcher
            .dispatch("org.gnome.Calendar", "list_events", &json!({}))
            .unwrap_err();
        assert!(err.error.contains("org.gnome.Calendar"), "{err}");
        assert!(err.error.contains("connect"), "{err}");
    }

    #[test]
    fn server_silence_times_out_instead_of_hanging_the_bus() {
        let dir = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(dir.path().join(format!("{APP}.sock"))).unwrap();
        let dispatcher = McpDispatcher::new(dir.path()).with_timeout(Duration::from_millis(100));
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_secs(5)); // never answers
        });
        let err = dispatcher
            .dispatch(APP, "list_events", &json!({}))
            .unwrap_err();
        assert!(err.error.contains("timed out"), "{err}");
        drop(server); // detached; the sleeping thread exits with the test process
    }
}
