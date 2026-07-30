//! The same corpus, through the surface a caller can actually reach
//! (issue #116).
//!
//! `gate.rs` drives `ContextStore::search_scoped` — the ACL-enforcing
//! function. That is the right property to prove, and it was the *only*
//! thing proven: the deployed path is `dev.lisaos.Context1.Search`,
//! which takes an option dictionary from the caller and used to pick an
//! UNFILTERED search whenever `scopes` was absent (#100). So the gate
//! could be green while the shipping overlay read mail chunks with no
//! grant at all — and it was, because the overlay sent no scopes.
//!
//! A suite that blocks merges on "0 cross-scope leaks" should fail when
//! the path a caller can reach leaks, not only when the path they may
//! choose does.
//!
//! So: the same corpus, over a real p2p connection to the real
//! interface, with the option dictionary itself fuzzed — because the
//! dictionary is the attack surface. A p2p peer cannot be identified,
//! which is precisely the caller this must be strictest with.

use lisa_acl_fuzz::{cases, expected_provenance, seeded_store};
use std::collections::HashMap;
use std::sync::Arc;
use zbus::zvariant::{OwnedValue, Value};

async fn client(
    store: Arc<lisa_contextd::ContextStore>,
    dir: &std::path::Path,
) -> zbus::Connection {
    let ledger = Arc::new(lisa_ledger::Ledger::open(dir.join("ledger.db")).unwrap());
    let (client_sock, server_sock) = tokio::net::UnixStream::pair().unwrap();
    let guid = zbus::Guid::generate();
    let server = zbus::connection::Builder::unix_stream(server_sock)
        .server(guid)
        .unwrap()
        .p2p()
        .serve_at(
            "/dev/lisaos/Context1",
            lisa_contextd::dbus::Context1::new(store, ledger),
        )
        .unwrap()
        .build();
    let client = zbus::connection::Builder::unix_stream(client_sock)
        .p2p()
        .build();
    let (server, client) = tokio::try_join!(server, client).unwrap();
    // The server must outlive the calls; leak it for the test's life.
    std::mem::forget(server);
    client
}

fn owned(v: Value<'_>) -> OwnedValue {
    OwnedValue::try_from(v).unwrap()
}

/// Every option shape a caller can send, including the ones that used
/// to select an unfiltered path.
fn option_shapes(scopes: &[String]) -> Vec<(&'static str, HashMap<String, OwnedValue>)> {
    let strv: Vec<&str> = scopes.iter().map(String::as_str).collect();
    let with = |extra: Vec<(&str, OwnedValue)>| {
        let mut m = HashMap::new();
        m.insert("scopes".to_string(), owned(Value::from(strv.clone())));
        for (k, v) in extra {
            m.insert(k.to_string(), v);
        }
        m
    };
    vec![
        ("scoped", with(vec![])),
        ("scoped+hybrid", with(vec![("hybrid", owned(true.into()))])),
        ("scoped+limit", with(vec![("limit", owned(50u32.into()))])),
        // The shapes that used to mean "everything": no scopes key at
        // all, and hybrid without scopes.
        ("no-scopes-key", HashMap::new()),
        (
            "hybrid-no-scopes",
            HashMap::from([("hybrid".to_string(), owned(true.into()))]),
        ),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn no_option_dictionary_yields_a_provenance_the_grant_forbids() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(seeded_store(&dir.path().join("ctx.db")));
    let conn = client(Arc::clone(&store), dir.path()).await;
    let proxy = zbus::Proxy::new(
        &conn,
        "dev.lisaos.Context1",
        "/dev/lisaos/Context1",
        "dev.lisaos.Context1",
    )
    .await
    .unwrap();

    // The bus round-trip is far slower than a direct call, so this runs
    // a slice of the corpus rather than all of it — chosen by stride so
    // it spans every query family and every scope shape rather than the
    // first N of one.
    let all = cases();
    let stride = (all.len() / 400).max(1);
    let mut leaks: Vec<String> = Vec::new();
    let mut hits_seen = 0usize;
    let mut calls = 0usize;

    for (query, scopes) in all.iter().step_by(stride) {
        for (shape, options) in option_shapes(scopes) {
            calls += 1;
            let Ok(reply) = proxy
                .call_method("Search", &(query.as_str(), options))
                .await
            else {
                continue;
            };
            let (json,): (String,) = reply.body().deserialize().unwrap();
            let hits: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
            // What the CALLER asked for is the ceiling. An option
            // dictionary must never widen it — and when it names no
            // scopes, the ceiling is nothing.
            let refs: Vec<&str> = scopes.iter().map(String::as_str).collect();
            let allowed = if shape.starts_with("scoped") {
                expected_provenance(&refs)
            } else {
                Default::default()
            };
            for h in hits {
                hits_seen += 1;
                let p = h["provenance"].as_str().unwrap_or_default().to_string();
                if !allowed.contains(&p) {
                    leaks.push(format!(
                        "  [{shape}] scopes {scopes:?} (allow {allowed:?}) query {query:?} -> {p}"
                    ));
                }
            }
        }
    }

    assert!(
        leaks.is_empty(),
        "{} leak(s) through the reachable surface across {calls} calls:\n{}",
        leaks.len(),
        leaks
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    // Non-vacuity: the scoped shapes must actually return things, or
    // "no leaks" is the empty-result tautology again.
    assert!(
        hits_seen >= 200,
        "only {hits_seen} hits across {calls} bus calls — the reachable surface \
         is not being exercised"
    );
}
