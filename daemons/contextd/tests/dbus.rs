//! dev.lisaos.Context1 over zbus peer-to-peer connections (PLAN §5.3).
//! P2P over a socketpair needs no bus daemon, so this runs on macOS dev
//! hosts and CI alike; real session-bus registration is exercised on
//! Linux systems.

use lisa_contextd::ContextStore;
use lisa_contextd::dbus::Context1;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use zbus::zvariant::OwnedValue;

struct Fixture {
    _dir: tempfile::TempDir,
    _server: zbus::Connection,
    client: zbus::Connection,
    ledger: Arc<lisa_ledger::Ledger>,
}

/// Store with mixed-provenance documents (the ACL boundary's test bed)
/// behind a p2p-served Context1, plus the ledger it gates on.
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ContextStore::open(dir.path().join("ctx.db")).unwrap());
    store
        .add_document(
            "/docs/report.md",
            "file",
            "quarterly revenue report: budget and forecast numbers",
        )
        .unwrap();
    store
        .add_document(
            "mail://inbox/42",
            "mail",
            "Re: budget — the revenue forecast looks off, can we talk",
        )
        .unwrap();
    let ledger = Arc::new(lisa_ledger::Ledger::open(dir.path().join("ledger.db")).unwrap());

    let (client_sock, server_sock) = tokio::net::UnixStream::pair().unwrap();
    let guid = zbus::Guid::generate();
    let server_fut = zbus::connection::Builder::unix_stream(server_sock)
        .server(guid)
        .unwrap()
        .p2p()
        .serve_at(
            "/dev/lisaos/Context1",
            Context1::new(store, Arc::clone(&ledger)),
        )
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
        ledger,
    }
}

async fn proxy(client: &zbus::Connection) -> zbus::Proxy<'_> {
    zbus::Proxy::new(
        client,
        "dev.lisaos.Context1",
        "/dev/lisaos/Context1",
        "dev.lisaos.Context1",
    )
    .await
    .unwrap()
}

fn owned(v: zbus::zvariant::Value<'_>) -> OwnedValue {
    OwnedValue::try_from(v).unwrap()
}

async fn search(
    p: &zbus::Proxy<'_>,
    query: &str,
    options: HashMap<String, OwnedValue>,
) -> Vec<Value> {
    let reply = p.call_method("Search", &(query, options)).await.unwrap();
    let (hits_json,): (String,) = reply.body().deserialize().unwrap();
    serde_json::from_str::<Value>(&hits_json)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn ping_reports_the_daemon() {
    let f = fixture().await;
    let p = proxy(&f.client).await;
    let reply = p.call_method("Ping", &()).await.unwrap();
    let (pong,): (String,) = reply.body().deserialize().unwrap();
    assert!(pong.starts_with("lisa-contextd "), "{pong}");
}

#[tokio::test]
async fn search_returns_hits_and_ledgers_first() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    let hits = search(&p, "budget revenue forecast", HashMap::new()).await;
    assert_eq!(hits.len(), 2, "both provenances match unscoped: {hits:?}");
    for h in &hits {
        assert!(h["source"].is_string() && h["provenance"].is_string());
        assert!(h["snippet"].is_string() && h["score"].is_number());
    }

    // The retrieval is ledgered — query hash, not text (PLAN §5.3).
    assert_eq!(f.ledger.count().unwrap(), 1);
    let entry = &f.ledger.tail(1).unwrap()[0];
    assert_eq!(entry.kind, "context.search");
    assert_eq!(
        entry.input_hash,
        blake3::hash(b"budget revenue forecast")
            .to_hex()
            .to_string()
    );

    // A zero-hit search is still a retrieval — still ledgered.
    assert!(search(&p, "xylophone", HashMap::new()).await.is_empty());
    assert_eq!(f.ledger.count().unwrap(), 2);
}

#[tokio::test]
async fn limit_and_hybrid_options_are_honored() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    let opts = HashMap::from([("limit".to_string(), owned(1u32.into()))]);
    assert_eq!(search(&p, "budget", opts).await.len(), 1);

    let opts = HashMap::from([("hybrid".to_string(), owned(true.into()))]);
    let hits = search(&p, "budget forecast", opts).await;
    assert!(!hits.is_empty(), "hybrid degrades to lexical sans vectors");
    assert_eq!(f.ledger.tail(1).unwrap()[0].kind, "context.search.hybrid");
}

#[tokio::test]
async fn scoped_search_enforces_the_acl_at_the_bus() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    // "budget revenue forecast" matches both docs; the documents scope
    // must still never see the mail chunk (0 cross-scope leaks, §5.3).
    let scopes = |s: &[&str]| {
        HashMap::from([(
            "scopes".to_string(),
            owned(s.iter().map(|s| s.to_string()).collect::<Vec<_>>().into()),
        )])
    };
    let hits = search(&p, "budget revenue forecast", scopes(&["documents.read"])).await;
    assert!(!hits.is_empty(), "the file doc should match");
    assert!(
        hits.iter().all(|h| h["provenance"] == "file"),
        "cross-scope leak: {hits:?}"
    );
    assert_eq!(f.ledger.tail(1).unwrap()[0].kind, "context.search.scoped");

    // An unrelated scope grants no provenance: deny by default.
    assert!(
        search(&p, "budget", scopes(&["inference"]))
            .await
            .is_empty()
    );

    // A present-but-EMPTY scopes list is still a scoped request: it must
    // deny everything, never widen into an unscoped search (issue #14).
    assert!(
        search(&p, "budget revenue forecast", scopes(&[]))
            .await
            .is_empty(),
        "empty scopes must match nothing"
    );
    assert_eq!(f.ledger.tail(1).unwrap()[0].kind, "context.search.scoped");
}

#[tokio::test]
async fn memory_roundtrip_is_namespace_isolated_and_wipe_is_total() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    for (app, val) in [("org.app.a", "dark"), ("org.app.b", "light")] {
        p.call_method("MemorySet", &(app, "theme", val))
            .await
            .unwrap();
    }

    let get = |app: &'static str, key: &'static str| {
        let p = p.clone();
        async move {
            p.call_method("MemoryGet", &(app, key))
                .await
                .map(|r| r.body().deserialize::<(String,)>().unwrap().0)
        }
    };
    assert_eq!(get("org.app.a", "theme").await.unwrap(), "dark");
    assert_eq!(get("org.app.b", "theme").await.unwrap(), "light");
    assert!(
        get("org.app.a", "missing").await.is_err(),
        "missing key errors"
    );

    let reply = p.call_method("MemoryList", &("org.app.a",)).await.unwrap();
    let (json,): (String,) = reply.body().deserialize().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&json).unwrap(),
        serde_json::json!({"theme": "dark"})
    );

    // Wipe app a; app b must be untouched (zero residual rows, §5.3).
    p.call_method("MemoryWipe", &("org.app.a",)).await.unwrap();
    assert!(get("org.app.a", "theme").await.is_err(), "wiped");
    let reply = p.call_method("MemoryList", &("org.app.a",)).await.unwrap();
    let (json,): (String,) = reply.body().deserialize().unwrap();
    assert_eq!(json, "{}");
    assert_eq!(get("org.app.b", "theme").await.unwrap(), "light");
}
