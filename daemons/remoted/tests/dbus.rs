//! dev.lisaos.Remote1 over zbus peer-to-peer connections — the Settings
//! app's management plane, exercised without a bus daemon so it runs on
//! macOS dev hosts and CI alike (same pattern as inferenced).
//!
//! # What p2p can and cannot say about #99
//!
//! A p2p peer carries no credentials: `lisa_peer::resolve` refuses to
//! ask a peer about itself (#133), so there is no uid and no pidfd, and
//! every mutating method refuses. That makes this file the natural home
//! for the *negative* half — an unidentified caller changes nothing —
//! and it is why the round-trip tests here now drive the broker
//! directly for setup.
//!
//! The positive half needs a real broker and a real executable, and
//! lives in `tests/bus.rs`.

use lisa_remoted::dbus::Remote1;
use lisa_remoted::service::Broker;
use std::sync::Arc;

async fn p2p_pair(broker: Arc<Broker>) -> (zbus::Connection, zbus::Connection) {
    let (client_sock, server_sock) = tokio::net::UnixStream::pair().unwrap();
    let guid = zbus::Guid::generate();
    let server_fut = zbus::connection::Builder::unix_stream(server_sock)
        .server(guid)
        .unwrap()
        .p2p()
        .serve_at("/dev/lisaos/Remote1", Remote1::new(broker))
        .unwrap()
        .build();
    let client_fut = zbus::connection::Builder::unix_stream(client_sock)
        .p2p()
        .build();
    let (server, client) = tokio::try_join!(server_fut, client_fut).unwrap();
    (server, client)
}

fn broker() -> (tempfile::TempDir, Arc<Broker>) {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Arc::new(lisa_ledger::Ledger::open(dir.path().join("ledger.db")).unwrap());
    let broker = Broker::open(&dir.path().join("state"), ledger).unwrap();
    (dir, broker)
}

async fn proxy(client: &zbus::Connection) -> zbus::Proxy<'static> {
    zbus::Proxy::new(
        client,
        "dev.lisaos.Remote1",
        "/dev/lisaos/Remote1",
        "dev.lisaos.Remote1",
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn state_reports_providers_and_default_deny_consent() {
    let (_dir, b) = broker();
    let (_server, client) = p2p_pair(b).await;
    let p = proxy(&client).await;

    let reply = p.call_method("State", &()).await.unwrap();
    let (raw,): (String,) = reply.body().deserialize().unwrap();
    let state: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(state["providers"].as_array().unwrap().len(), 14);
    assert_eq!(
        state["may_offload"]["prompt"], false,
        "nothing leaves by default"
    );
}

/// A `Manager` for setting state up. Goes through the real `authorize`
/// with the test binary in its own allowlist — the same shape as
/// Settings being in the shipped one.
fn as_manager() -> lisa_peer::manager::Manager {
    let me = std::env::current_exe().unwrap().canonicalize().unwrap();
    lisa_peer::manager::Manager::authorize(true, Some(&me), std::slice::from_ref(&me)).unwrap()
}

#[tokio::test]
async fn provider_and_key_management_round_trips() {
    let (_dir, b) = broker();
    let (_server, client) = p2p_pair(Arc::clone(&b)).await;
    let p = proxy(&client).await;

    // Set up through the broker: what this test is about is that State
    // reflects the rows and never leaks the key, not who may write them.
    let who = as_manager();
    b.add_provider(
        &who,
        "lab",
        "Lab",
        "https://lab.example/v1",
        lisa_remoted::net::Locality::PublicOnly,
    )
    .unwrap();
    b.set_key(&who, "lab", "lab-secret").unwrap();
    b.set_consent(&who, "prompt", true).unwrap();

    let (raw,): (String,) = p
        .call_method("State", &())
        .await
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    assert!(
        !raw.contains("lab-secret"),
        "keys are write-only over D-Bus"
    );
    let state: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let lab = state["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "lab")
        .unwrap()
        .clone();
    assert_eq!(lab["has_credential"], true);
    assert_eq!(state["may_offload"]["prompt"], true);

    b.remove_provider(&who, "lab").unwrap();
    assert!(
        !b.secrets().has("lab"),
        "removing a provider drops its credential"
    );
}

/// Issue #99 on the D-Bus plane. `dev.lisaos.Remote1` sits on the
/// session bus, which anything the user runs can reach; the filed
/// exploit turned on all six offload scopes, registered an attacker's
/// endpoint, and overwrote a credential — with no prompt and no
/// interaction.
///
/// A p2p caller has no credentials to offer, which is exactly the case
/// that has to fail closed.
#[tokio::test]
async fn an_unidentified_caller_changes_nothing() {
    let (_dir, b) = broker();
    let (_server, client) = p2p_pair(Arc::clone(&b)).await;
    let p = proxy(&client).await;

    for scope in ["prompt", "files", "mail", "calendar", "screen", "memory"] {
        assert!(
            p.call_method("SetConsent", &(scope, true)).await.is_err(),
            "{scope} was turned on by an unidentified caller"
        );
    }
    assert!(
        p.call_method(
            "AddProvider",
            &("sink", "Sink", "https://attacker.example/v1")
        )
        .await
        .is_err(),
        "an egress endpoint was registered by an unidentified caller"
    );
    assert!(
        p.call_method("SetKey", &("openai", "sk-attacker"))
            .await
            .is_err()
    );
    assert!(p.call_method("ClearKey", &("openai",)).await.is_err());
    assert!(p.call_method("RemoveProvider", &("openai",)).await.is_err());
    assert!(p.call_method("BeginLogin", &("anthropic",)).await.is_err());
    assert!(p.call_method("Logout", &("anthropic",)).await.is_err());

    // And nothing moved.
    let consent = b.consent_json();
    for scope in ["prompt", "files", "mail", "calendar", "screen", "memory"] {
        assert_eq!(consent["may_offload"][scope], false, "{scope} ended up on");
    }
    assert!(
        !b.providers_json()["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "sink"),
        "the sink provider was registered anyway"
    );

    // Reads stay open — Settings and the Assistant both render State,
    // and the model picker calls ListModels.
    assert!(p.call_method("State", &()).await.is_ok());
    assert!(p.call_method("Ping", &()).await.is_ok());
}

#[tokio::test]
async fn state_reports_oauth_capability_per_provider() {
    let (_dir, b) = broker();
    let (_server, client) = p2p_pair(b).await;
    let p = proxy(&client).await;

    let (raw,): (String,) = p
        .call_method("State", &())
        .await
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    let state: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let by_id = |id: &str| {
        state["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()
            .clone()
    };
    // anthropic + openai are OAuth-capable; nothing is connected yet.
    for id in ["anthropic", "openai"] {
        let row = by_id(id);
        assert_eq!(row["oauth_capable"], true, "{id} is oauth-capable");
        assert_eq!(row["connected"], false, "{id} not signed in yet");
        assert_eq!(row["auth"], "key", "{id} defaults to key mode");
    }
    // Everything else stays key-only.
    for id in ["tinker", "together", "google", "openrouter"] {
        assert_eq!(by_id(id)["oauth_capable"], false, "{id} key-only");
    }
}

#[tokio::test]
async fn begin_login_rejects_key_only_providers_and_logout_is_idempotent() {
    let (_dir, b) = broker();
    let who = as_manager();

    // A key-only provider cannot start OAuth (and binds no port). Driven
    // through the broker: starting a login is manager-only (#99), and
    // the p2p caller above is nobody. What is under test is the
    // capability check, not the identity one.
    let err = b.begin_login(&who, "tinker").await.unwrap_err();
    assert!(
        err.to_string().contains("does not support OAuth"),
        "got: {err}"
    );

    // Logout with no stored session is a clean no-op.
    b.logout(&who, "anthropic").unwrap();
}
