//! dev.lisaos.Remote1 over zbus peer-to-peer connections — the Settings
//! app's management plane, exercised without a bus daemon so it runs on
//! macOS dev hosts and CI alike (same pattern as inferenced).

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

#[tokio::test]
async fn provider_and_key_management_round_trips() {
    let (_dir, b) = broker();
    let (_server, client) = p2p_pair(Arc::clone(&b)).await;
    let p = proxy(&client).await;

    p.call_method("AddProvider", &("lab", "Lab", "https://lab.example/v1"))
        .await
        .unwrap();
    p.call_method("SetKey", &("lab", "lab-secret"))
        .await
        .unwrap();
    p.call_method("SetConsent", &("prompt", true))
        .await
        .unwrap();

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

    p.call_method("RemoveProvider", &("lab",)).await.unwrap();
    assert!(
        !b.secrets().has("lab"),
        "removing a provider drops its credential"
    );
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
    let (_server, client) = p2p_pair(b).await;
    let p = proxy(&client).await;

    // A key-only provider cannot start OAuth (and binds no port).
    let err = p.call_method("BeginLogin", &("tinker",)).await.unwrap_err();
    assert!(
        err.to_string().contains("does not support OAuth"),
        "got: {err}"
    );

    // Logout with no stored session is a clean no-op.
    p.call_method("Logout", &("anthropic",)).await.unwrap();
}
