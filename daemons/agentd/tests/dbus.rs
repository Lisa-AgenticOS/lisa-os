//! dev.lisaos.Agent1 over zbus peer-to-peer connections (PLAN §5.4).
//! P2P over a socketpair needs no bus daemon, so this runs on macOS dev
//! hosts and CI alike; real session-bus registration is exercised on
//! Linux systems.

use lisa_agentd::bus::{AgentBus, Dispatcher, RecordingDispatcher};
use lisa_agentd::dbus::Agent1;
use lisa_agentd::journal::UndoJournal;
use lisa_agentd::manifest::Manifest;
use lisa_agentd::registry::Registry;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use zbus::zvariant::OwnedValue;

fn calendar_manifest() -> Manifest {
    Manifest::from_json(
        &json!({
            "lisa_manifest": 1,
            "app_id": "org.gnome.Calendar",
            "mcp": { "transport": "unix", "activatable": true },
            "tools": [
                { "name": "add_event", "tier": "write",
                  "description": "Create a calendar event",
                  "input_schema": { "type": "object", "required": ["title", "start"],
                      "properties": { "title": {"type": "string"},
                                      "start": {"type": "string"} } },
                  "undo": { "tool": "delete_event",
                            "map": { "event_id": "$result.event_id" } } },
                { "name": "delete_event", "tier": "destructive",
                  "description": "Delete a calendar event",
                  "input_schema": { "type": "object", "required": ["event_id"],
                      "properties": { "event_id": {"type": "string"} } } },
                { "name": "list_events", "tier": "read",
                  "description": "List events",
                  "input_schema": { "type": "object", "properties": {} } }
            ]
        })
        .to_string(),
    )
    .unwrap()
}

struct Fixture {
    _dir: tempfile::TempDir,
    _server: zbus::Connection,
    client: zbus::Connection,
    dispatcher: Arc<RecordingDispatcher>,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Arc::new(lisa_ledger::Ledger::open(dir.path().join("ledger.db")).unwrap());
    let dispatcher = Arc::new(RecordingDispatcher::returning(json!({"event_id": "evt-1"})));
    let mut registry = Registry::new();
    registry.insert(calendar_manifest()).unwrap();
    let bus = Arc::new(AgentBus::new(
        registry,
        ledger,
        UndoJournal::open_in_memory().unwrap(),
        Arc::clone(&dispatcher) as Arc<dyn Dispatcher>,
    ));

    let (client_sock, server_sock) = tokio::net::UnixStream::pair().unwrap();
    let guid = zbus::Guid::generate();
    let server_fut = zbus::connection::Builder::unix_stream(server_sock)
        .server(guid)
        .unwrap()
        .p2p()
        .serve_at("/dev/lisaos/Agent1", Agent1::new(bus))
        .unwrap()
        .build();
    let client_fut = zbus::connection::Builder::unix_stream(client_sock)
        .p2p()
        .build();
    let (server, client) = tokio::try_join!(server_fut, client_fut).unwrap();
    Fixture {
        _dir: dir,
        _server: server,
        client,
        dispatcher,
    }
}

async fn proxy(client: &zbus::Connection) -> zbus::Proxy<'_> {
    zbus::Proxy::new(
        client,
        "dev.lisaos.Agent1",
        "/dev/lisaos/Agent1",
        "dev.lisaos.Agent1",
    )
    .await
    .unwrap()
}

fn options(provenance: &[&str]) -> HashMap<String, OwnedValue> {
    let chain: Vec<String> = provenance.iter().map(|s| s.to_string()).collect();
    HashMap::from([(
        "provenance".to_string(),
        OwnedValue::try_from(zbus::zvariant::Value::from(chain)).unwrap(),
    )])
}

async fn request_call(
    proxy: &zbus::Proxy<'_>,
    tool: &str,
    args: Value,
    provenance: &[&str],
) -> (u64, String, String) {
    proxy
        .call_method(
            "RequestCall",
            &(
                "org.gnome.Calendar",
                tool,
                args.to_string(),
                options(provenance),
            ),
        )
        .await
        .unwrap()
        .body()
        .deserialize()
        .unwrap()
}

#[tokio::test]
async fn list_and_discover_report_registered_tools() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    let reply = p.call_method("ListTools", &()).await.unwrap();
    let (tools_json,): (String,) = reply.body().deserialize().unwrap();
    let tools: Value = serde_json::from_str(&tools_json).unwrap();
    assert_eq!(tools.as_array().unwrap().len(), 3);

    let reply = p
        .call_method("Discover", &("add a calendar event",))
        .await
        .unwrap();
    let (hits_json,): (String,) = reply.body().deserialize().unwrap();
    let hits: Value = serde_json::from_str(&hits_json).unwrap();
    assert_eq!(hits[0]["name"], "add_event");
}

#[tokio::test]
async fn read_call_with_user_provenance_executes() {
    let f = fixture().await;
    let p = proxy(&f.client).await;
    let (_, disposition, detail) = request_call(&p, "list_events", json!({}), &["user"]).await;
    assert_eq!(disposition, "executed", "{detail}");
    assert_eq!(f.dispatcher.dispatched(), 1);
}

#[tokio::test]
async fn write_call_parks_then_confirm_executes_and_undo_reverts() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    let (call_id, disposition, spec_json) = request_call(
        &p,
        "add_event",
        json!({"title": "dentist", "start": "2026-07-24"}),
        &["user"],
    )
    .await;
    assert_eq!(disposition, "confirm-chip");
    let spec: Value = serde_json::from_str(&spec_json).unwrap();
    assert_eq!(spec["escalated"], false);
    assert_eq!(f.dispatcher.dispatched(), 0, "nothing runs before consent");

    let reply = p.call_method("Confirm", &(call_id, true)).await.unwrap();
    let (status, _detail): (String, String) = reply.body().deserialize().unwrap();
    assert_eq!(status, "executed");
    assert_eq!(f.dispatcher.dispatched(), 1);

    let reply = p.call_method("Undo", &()).await.unwrap();
    let (report_json,): (String,) = reply.body().deserialize().unwrap();
    let report: Value = serde_json::from_str(&report_json).unwrap();
    assert_eq!(report["status"], "undone", "{report_json}");
    assert_eq!(report["undo_tool"], "delete_event");
    let calls = f.dispatcher.calls.lock().unwrap();
    assert_eq!(calls[1].2, json!({"event_id": "evt-1"}));
}

#[tokio::test]
async fn untrusted_provenance_escalates_over_dbus() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    // Read + mail-tainted chain → chip, not silent.
    let (_, disposition, spec_json) =
        request_call(&p, "list_events", json!({}), &["user", "mail"]).await;
    assert_eq!(disposition, "confirm-chip", "{spec_json}");

    // Missing provenance → unknown origin → escalates too.
    let reply = p
        .call_method(
            "RequestCall",
            &(
                "org.gnome.Calendar",
                "list_events",
                "{}",
                HashMap::<String, OwnedValue>::new(),
            ),
        )
        .await
        .unwrap();
    let (_, disposition, _): (u64, String, String) = reply.body().deserialize().unwrap();
    assert_eq!(disposition, "confirm-chip");
    assert_eq!(f.dispatcher.dispatched(), 0);
}

#[tokio::test]
async fn deny_refuses_and_bad_args_are_invalid() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    let (call_id, disposition, _) =
        request_call(&p, "delete_event", json!({"event_id": "evt-1"}), &["user"]).await;
    assert_eq!(disposition, "confirm-modal");
    let reply = p.call_method("Confirm", &(call_id, false)).await.unwrap();
    let (status, _): (String, String) = reply.body().deserialize().unwrap();
    assert_eq!(status, "denied");
    assert_eq!(f.dispatcher.dispatched(), 0);

    let err = p
        .call_method(
            "RequestCall",
            &(
                "org.gnome.Calendar",
                "list_events",
                "not json",
                HashMap::<String, OwnedValue>::new(),
            ),
        )
        .await;
    assert!(err.is_err(), "malformed args_json must be rejected");
}
