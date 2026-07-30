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
    store: Arc<ContextStore>,
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
            Context1::new(Arc::clone(&store), Arc::clone(&ledger)),
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
        store,
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

/// Issue #100. This test used to read `assert_eq!(hits.len(), 2, "both
/// provenances match unscoped")` — it pinned the bug as the contract.
/// Omitting one dictionary key returned mail and screen chunks to any
/// peer on the session bus.
///
/// A p2p caller cannot be identified, so it is not the user's own
/// tooling, so it gets the scoped path with the scopes it asked for —
/// here, none.
#[tokio::test]
async fn a_search_with_no_scopes_returns_nothing_and_is_still_ledgered() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    let hits = search(&p, "budget revenue forecast", HashMap::new()).await;
    assert!(
        hits.is_empty(),
        "an unscoped search read the index anyway: {hits:?}"
    );

    // Still a retrieval, still ledgered — query hash, not text (§5.3).
    assert_eq!(f.ledger.count().unwrap(), 1);
    let entry = &f.ledger.tail(1).unwrap()[0];
    assert_eq!(entry.kind, "context.search.scoped");
    assert_eq!(
        entry.input_hash,
        blake3::hash(b"budget revenue forecast")
            .to_hex()
            .to_string()
    );
    // And it names who asked and with what, which §5.3 asks for and the
    // old `app_id: "host"` could not provide.
    assert!(entry.app_id.starts_with("host:"), "{:?}", entry.app_id);
    assert!(entry.detail.contains("scopes"), "{:?}", entry.detail);

    // A zero-hit search is still a retrieval — still ledgered.
    assert!(search(&p, "xylophone", HashMap::new()).await.is_empty());
    assert_eq!(f.ledger.count().unwrap(), 2);
}

/// The shape of a hit is still the contract — asserted where hits
/// actually come back.
#[tokio::test]
async fn scoped_hits_carry_source_provenance_snippet_and_score() {
    let f = fixture().await;
    let p = proxy(&f.client).await;
    let mut opts = HashMap::new();
    opts.insert(
        "scopes".to_string(),
        owned(zbus::zvariant::Value::from(vec!["documents.read"])),
    );
    let hits = search(&p, "budget revenue forecast", opts).await;
    assert!(!hits.is_empty());
    for h in &hits {
        assert!(h["source"].is_string() && h["provenance"].is_string());
        assert!(h["snippet"].is_string() && h["score"].is_number());
        assert_eq!(h["provenance"], "file");
    }
}

/// `limit` and `hybrid` are ranking options and must survive the ACL —
/// an app that asked for the better ranking and silently got the worse
/// one would have no way to tell.
#[tokio::test]
async fn limit_and_hybrid_options_are_honored_within_scope() {
    let f = fixture().await;
    let p = proxy(&f.client).await;
    let docs = || {
        owned(zbus::zvariant::Value::from(vec![
            "documents.read",
            "mail.read",
        ]))
    };

    let opts = HashMap::from([
        ("limit".to_string(), owned(1u32.into())),
        ("scopes".to_string(), docs()),
    ]);
    assert_eq!(search(&p, "budget", opts).await.len(), 1);

    let opts = HashMap::from([
        ("hybrid".to_string(), owned(true.into())),
        ("scopes".to_string(), docs()),
    ]);
    let hits = search(&p, "budget forecast", opts).await;
    assert!(!hits.is_empty(), "hybrid degrades to lexical sans vectors");
    assert_eq!(
        f.ledger.tail(1).unwrap()[0].kind,
        "context.search.scoped.hybrid"
    );

    // And the ACL still holds under the hybrid ranking: a disallowed
    // chunk must not be rerankable into the answer.
    let opts = HashMap::from([
        ("hybrid".to_string(), owned(true.into())),
        (
            "scopes".to_string(),
            owned(zbus::zvariant::Value::from(vec!["documents.read"])),
        ),
    ]);
    let hits = search(&p, "budget revenue forecast", opts).await;
    assert!(!hits.is_empty());
    assert!(
        hits.iter().all(|h| h["provenance"] == "file"),
        "hybrid reranked a mail chunk into a documents-only read: {hits:?}"
    );
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

/// Issue #101. This test used to be the demonstration: one connection
/// wrote, read and wiped two different namespaces by naming them. The
/// assistant persists whole session transcripts in its namespace, so
/// `MemoryList("app.lisaos.Assistant")` was a transcript dump, and
/// `MemoryWipe` on it was destruction.
///
/// A caller may now only name its own namespace. A p2p peer cannot be
/// identified, so its namespace is `host:unknown` — a real namespace,
/// shared by every unidentifiable caller, not a pass.
#[tokio::test]
async fn a_caller_cannot_touch_another_apps_namespace() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    for verb in ["MemoryGet", "MemoryList", "MemoryWipe"] {
        let call = match verb {
            "MemoryGet" => {
                p.call_method(verb, &("app.lisaos.Assistant", "sessions"))
                    .await
            }
            _ => p.call_method(verb, &("app.lisaos.Assistant",)).await,
        };
        assert!(call.is_err(), "{verb} reached another app's namespace");
    }
    assert!(
        p.call_method("MemorySet", &("app.lisaos.Assistant", "k", "v"))
            .await
            .is_err(),
        "MemorySet wrote into another app's namespace"
    );
}

/// The namespace still works — for the caller's own. An empty `app`
/// argument means "mine", which is what an app should send.
#[tokio::test]
async fn memory_roundtrip_in_the_callers_own_namespace_and_wipe_is_total() {
    let f = fixture().await;
    let p = proxy(&f.client).await;

    p.call_method("MemorySet", &("", "theme", "dark"))
        .await
        .unwrap();
    let get = |key: &'static str| {
        let p = p.clone();
        async move {
            p.call_method("MemoryGet", &("", key))
                .await
                .map(|r| r.body().deserialize::<(String,)>().unwrap().0)
        }
    };
    assert_eq!(get("theme").await.unwrap(), "dark");
    assert!(get("missing").await.is_err(), "missing key errors");

    let reply = p.call_method("MemoryList", &("",)).await.unwrap();
    let (json,): (String,) = reply.body().deserialize().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&json).unwrap(),
        serde_json::json!({"theme": "dark"})
    );

    // Another namespace, written directly, must survive our wipe (zero
    // residual rows for us, nothing touched for them — §5.3).
    f.store.memory_set("org.app.b", "theme", "light").unwrap();
    p.call_method("MemoryWipe", &("",)).await.unwrap();
    assert!(get("theme").await.is_err(), "wiped");
    let reply = p.call_method("MemoryList", &("",)).await.unwrap();
    let (json,): (String,) = reply.body().deserialize().unwrap();
    assert_eq!(json, "{}");
    assert_eq!(
        f.store.memory_get("org.app.b", "theme").unwrap().as_deref(),
        Some("light"),
        "a wipe crossed into another namespace"
    );
}
