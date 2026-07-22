//! The portal over zbus peer-to-peer connections (PLAN §5.5). P2p over
//! a socketpair needs no bus daemon, so the whole trust boundary —
//! consent, grants, quotas, Ledger attribution, revocation killing live
//! sessions — runs on macOS dev hosts and CI alike. The §5.5 acceptance
//! items that need a real desktop (Flatpak sandbox, consent dialog
//! pixels) are exercised on Linux systems.

use lisa_portal::consent::{ConsentUi, StaticConsent};
use lisa_portal::grants::{Effective, GrantStore};
use lisa_portal::identity::{AppIdentity, StaticIdentity};
use lisa_portal::portal::{PORTAL_PATH, PortalState, serve_on_builder};
use lisa_portal::quota::QuotaConfig;
use lisa_portal::upstream::stub::StubUpstream;
use lisa_portal::upstream::{InferenceUpstream, ZbusUpstream};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

struct Harness {
    #[allow(dead_code)]
    server: zbus::Connection,
    client: zbus::Connection,
    grants: Arc<GrantStore>,
    ledger: Arc<lisa_ledger::Ledger>,
    #[allow(dead_code)]
    ledger_dir: tempfile::TempDir,
}

async fn harness_with(
    identity: AppIdentity,
    consent: Arc<dyn ConsentUi>,
    upstream: Arc<dyn InferenceUpstream>,
    quota: QuotaConfig,
) -> Harness {
    let grants = Arc::new(GrantStore::open_in_memory().unwrap());
    let ledger_dir = tempfile::tempdir().unwrap();
    let ledger = Arc::new(lisa_ledger::Ledger::open(ledger_dir.path().join("ledger.db")).unwrap());
    let state = PortalState::new(
        Arc::new(StaticIdentity(identity)),
        consent,
        upstream,
        Arc::clone(&grants),
        Arc::clone(&ledger),
        quota,
    );

    let (client_sock, server_sock) = tokio::net::UnixStream::pair().unwrap();
    let guid = zbus::Guid::generate();
    let server_builder = zbus::connection::Builder::unix_stream(server_sock)
        .server(guid)
        .unwrap()
        .p2p();
    let server_fut = serve_on_builder(server_builder, Arc::clone(&state))
        .unwrap()
        .build();
    let client_fut = zbus::connection::Builder::unix_stream(client_sock)
        .p2p()
        .build();
    let (server, client) = tokio::try_join!(server_fut, client_fut).unwrap();
    Harness {
        server,
        client,
        grants,
        ledger,
        ledger_dir,
    }
}

async fn harness(identity: AppIdentity, consent: Arc<dyn ConsentUi>) -> Harness {
    harness_with(
        identity,
        consent,
        Arc::new(StubUpstream),
        QuotaConfig::default(),
    )
    .await
}

async fn portal_proxy(h: &Harness) -> zbus::Proxy<'static> {
    zbus::Proxy::new(
        &h.client,
        "org.lisa.Portal",
        PORTAL_PATH,
        "org.lisa.portal.Inference",
    )
    .await
    .unwrap()
}

async fn grants_proxy(h: &Harness) -> zbus::Proxy<'static> {
    zbus::Proxy::new(
        &h.client,
        "org.lisa.Portal",
        PORTAL_PATH,
        "org.lisa.portal.Grants",
    )
    .await
    .unwrap()
}

async fn open_session(h: &Harness) -> zbus::Result<(OwnedObjectPath, std::os::fd::OwnedFd)> {
    let proxy = portal_proxy(h).await;
    let reply = proxy
        .call_method("OpenSession", &(HashMap::<String, OwnedValue>::new(),))
        .await?;
    let (path, fd): (OwnedObjectPath, zbus::zvariant::OwnedFd) =
        reply.body().deserialize().unwrap();
    Ok((path, fd.into()))
}

async fn session_proxy(h: &Harness, path: OwnedObjectPath) -> zbus::Proxy<'static> {
    zbus::Proxy::new(
        &h.client,
        "org.lisa.Portal",
        path,
        "org.lisa.portal.Session",
    )
    .await
    .unwrap()
}

fn read_to_eof(fd: std::os::fd::OwnedFd) -> tokio::task::JoinHandle<String> {
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::from(fd);
        let mut s = String::new();
        file.read_to_string(&mut s).unwrap();
        s
    })
}

#[tokio::test]
async fn first_use_without_consent_backend_is_denied() {
    let h = harness(
        AppIdentity::flatpak("org.example.Demo"),
        Arc::new(StaticConsent::unavailable()),
    )
    .await;
    let err = open_session(&h).await.expect_err("must be denied");
    assert!(
        err.to_string().contains("AccessDenied"),
        "fail closed without a dialog backend: {err}"
    );
    // The refusal is ledgered under the real app id.
    let tail = h.ledger.tail(10).unwrap();
    assert_eq!(tail[0].kind, "context.grant");
    assert_eq!(tail[0].status, "denied");
    assert_eq!(tail[0].app_id, "org.example.Demo");
}

#[tokio::test]
async fn zero_permission_app_gets_a_session_only_after_user_grant() {
    // §5.5 acceptance: session only after grant. Consent answers
    // "always" → session opens, grant persists, generate streams over
    // the fd, and every Ledger entry carries the Flatpak app id.
    let h = harness(
        AppIdentity::flatpak("org.example.Demo"),
        Arc::new(StaticConsent::allow_always()),
    )
    .await;
    let (path, fd) = open_session(&h).await.unwrap();
    assert_eq!(
        h.grants.effective("org.example.Demo", "inference").unwrap(),
        Effective::Allowed
    );

    let session = session_proxy(&h, path).await;
    session
        .call_method(
            "Generate",
            &(
                "hello through the portal",
                HashMap::<String, OwnedValue>::new(),
            ),
        )
        .await
        .unwrap();
    let text = read_to_eof(fd).await.unwrap();
    assert!(
        text.contains("hello through the portal"),
        "streamed: {text}"
    );

    let tail = h.ledger.tail(10).unwrap();
    let kinds: Vec<&str> = tail.iter().map(|e| e.kind.as_str()).collect();
    assert!(kinds.contains(&"context.grant"));
    assert!(kinds.contains(&"inference.session"));
    assert!(kinds.contains(&"inference.generate"));
    assert!(
        tail.iter().all(|e| e.app_id == "org.example.Demo"),
        "every entry carries the app id: {tail:?}"
    );
}

#[tokio::test]
async fn only_this_time_grants_without_persisting() {
    let h = harness(
        AppIdentity::flatpak("org.example.Demo"),
        Arc::new(StaticConsent::allow_once()),
    )
    .await;
    open_session(&h).await.unwrap();
    assert_eq!(
        h.grants.effective("org.example.Demo", "inference").unwrap(),
        Effective::Unset,
        "allow-once must not persist"
    );
}

#[tokio::test]
async fn remembered_deny_refuses_without_reprompting() {
    let h = harness(
        AppIdentity::flatpak("org.example.Demo"),
        // If the portal re-prompted, this backend would say yes — the
        // remembered deny must win without asking.
        Arc::new(StaticConsent::allow_always()),
    )
    .await;
    h.grants
        .record(
            "org.example.Demo",
            "inference",
            lisa_portal::grants::GrantAction::Deny,
        )
        .unwrap();
    let err = open_session(&h).await.expect_err("remembered deny");
    assert!(err.to_string().contains("AccessDenied"));
}

#[tokio::test]
async fn host_identity_is_attributed_in_the_ledger() {
    // §5.5 acceptance: correct app-id under host execution too.
    let h = harness(
        AppIdentity::host("host:vim"),
        Arc::new(StaticConsent::allow_always()),
    )
    .await;
    open_session(&h).await.unwrap();
    let tail = h.ledger.tail(10).unwrap();
    assert!(tail.iter().all(|e| e.app_id == "host:vim"));
    assert!(
        tail.iter()
            .any(|e| e.kind == "inference.session" && e.detail.contains("identity=host"))
    );
}

#[tokio::test]
async fn revoke_kills_the_live_session_and_next_use_reprompts() {
    // §5.5 acceptance: revoking kills the live session < 1 s.
    let h = harness(
        AppIdentity::host("host:demo"),
        Arc::new(StaticConsent::allow_always()),
    )
    .await;
    let (path, fd) = open_session(&h).await.unwrap();
    let session = session_proxy(&h, path).await;
    let reader = read_to_eof(fd);

    let started = std::time::Instant::now();
    let grants = grants_proxy(&h).await;
    let reply = grants
        .call_method("Revoke", &("host:demo", "inference"))
        .await
        .unwrap();
    let (killed,): (u32,) = reply.body().deserialize().unwrap();
    assert_eq!(killed, 1);

    // The daemon side dropped its pipe writer → the app's fd sees EOF...
    reader.await.unwrap();
    // ...and the portal session object is gone.
    let err = session.call_method("Cancel", &()).await;
    assert!(err.is_err(), "session must be dead after revoke");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "revocation must land in under a second"
    );

    // Post-revoke state is unset: the next request prompts again.
    assert_eq!(
        h.grants.effective("host:demo", "inference").unwrap(),
        Effective::Unset
    );
    let tail = h.ledger.tail(10).unwrap();
    assert!(
        tail.iter()
            .any(|e| e.kind == "context.grant" && e.status == "revoked")
    );
}

#[tokio::test]
async fn request_rate_quota_refuses_the_excess_call() {
    let h = harness_with(
        AppIdentity::host("host:loop"),
        Arc::new(StaticConsent::allow_always()),
        Arc::new(StubUpstream),
        QuotaConfig {
            requests_per_min: 2,
            tokens_per_day: 1_000_000,
        },
    )
    .await;
    let (path, _fd) = open_session(&h).await.unwrap();
    let session = session_proxy(&h, path).await;
    for _ in 0..2 {
        session.call_method("Embed", &(vec!["x"],)).await.unwrap();
    }
    let err = session
        .call_method("Embed", &(vec!["x"],))
        .await
        .expect_err("third request in the window must hit the quota");
    assert!(err.to_string().contains("LimitsExceeded"), "{err}");
}

#[tokio::test]
async fn token_budget_quota_refuses_once_spent() {
    let h = harness_with(
        AppIdentity::host("host:hog"),
        Arc::new(StaticConsent::allow_always()),
        Arc::new(StubUpstream),
        QuotaConfig {
            requests_per_min: 1000,
            tokens_per_day: 5,
        },
    )
    .await;
    let (path, _fd) = open_session(&h).await.unwrap();
    let session = session_proxy(&h, path).await;
    // Six words spend the 5-token budget.
    session
        .call_method("Embed", &(vec!["one two three four five six"],))
        .await
        .unwrap();
    let err = session
        .call_method("Embed", &(vec!["more"],))
        .await
        .expect_err("budget is spent");
    assert!(err.to_string().contains("LimitsExceeded"), "{err}");
}

#[tokio::test]
async fn grants_management_is_refused_to_sandboxed_callers() {
    // The resolver says every caller is a Flatpak app — so even the
    // management surface must refuse (apps cannot grant themselves).
    let h = harness(
        AppIdentity::flatpak("org.example.Demo"),
        Arc::new(StaticConsent::unavailable()),
    )
    .await;
    let grants = grants_proxy(&h).await;
    let err = grants
        .call_method("Grant", &("org.example.Demo", "inference"))
        .await
        .expect_err("sandboxed callers cannot manage grants");
    assert!(err.to_string().contains("AccessDenied"));
}

#[tokio::test]
async fn settings_grant_pre_authorizes_without_a_prompt() {
    let h = harness(
        AppIdentity::host("host:settings-demo"),
        // Consent backend absent: only the pre-grant can authorize.
        Arc::new(StaticConsent::unavailable()),
    )
    .await;
    let grants = grants_proxy(&h).await;
    grants
        .call_method("Grant", &("host:settings-demo", "inference"))
        .await
        .unwrap();
    open_session(&h)
        .await
        .expect("pre-granted app opens with no prompt");

    let reply = grants.call_method("List", &()).await.unwrap();
    let (rows,): (Vec<(String, String, String)>,) = reply.body().deserialize().unwrap();
    assert_eq!(
        rows,
        vec![(
            "host:settings-demo".to_string(),
            "inference".to_string(),
            "allowed".to_string()
        )]
    );
}

#[tokio::test]
async fn every_session_open_is_preceded_by_a_ledger_entry() {
    // No ledger entry, no inference (PLAN §4 rule 4): the session-start
    // entry must exist by the time OpenSession returns.
    let h = harness(
        AppIdentity::host("host:demo"),
        Arc::new(StaticConsent::allow_always()),
    )
    .await;
    assert_eq!(h.ledger.count().unwrap(), 0);
    open_session(&h).await.unwrap();
    let tail = h.ledger.tail(10).unwrap();
    assert!(
        tail.iter()
            .any(|e| e.kind == "inference.session" && e.status == "started")
    );
}

#[tokio::test]
async fn portal_proxies_to_the_real_inferenced_interface() {
    // End-to-end over two p2p hops: app ↔ portal ↔ org.lisa.Inference1
    // (the real interface from daemons/inferenced, stub engine).
    use lisa_inferenced::dbus::Inference1;
    use lisa_inferenced::engine::StubEngine;
    use lisa_inferenced::scheduler::Scheduler;

    let (client_sock, server_sock) = tokio::net::UnixStream::pair().unwrap();
    let guid = zbus::Guid::generate();
    let daemon_fut = zbus::connection::Builder::unix_stream(server_sock)
        .server(guid)
        .unwrap()
        .p2p()
        .serve_at(
            "/org/lisa/Inference1",
            Inference1::new(
                Arc::new(lisa_inferenced::pool::SingleEngine {
                    engine: Arc::new(StubEngine),
                    name: "lisa-system-stub".to_string(),
                }),
                Arc::new(Scheduler::new(1)),
            ),
        )
        .unwrap()
        .build();
    let upstream_fut = zbus::connection::Builder::unix_stream(client_sock)
        .p2p()
        .build();
    let (_daemon, upstream_conn) = tokio::try_join!(daemon_fut, upstream_fut).unwrap();

    let h = harness_with(
        AppIdentity::flatpak("org.example.Demo"),
        Arc::new(StaticConsent::allow_always()),
        Arc::new(ZbusUpstream::new(upstream_conn)),
        QuotaConfig::default(),
    )
    .await;
    let (path, fd) = open_session(&h).await.unwrap();
    let session = session_proxy(&h, path).await;
    session
        .call_method(
            "Generate",
            &("end to end", HashMap::<String, OwnedValue>::new()),
        )
        .await
        .unwrap();
    let text = read_to_eof(fd).await.unwrap();
    assert!(
        text.contains("end to end"),
        "tokens flowed daemon → portal fd → app: {text}"
    );

    let reply = session
        .call_method("Embed", &(vec!["alpha", "beta"],))
        .await
        .unwrap();
    let (vectors,): (Vec<Vec<f64>>,) = reply.body().deserialize().unwrap();
    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0].len(), 8, "inferenced's stub embedding dims");
}
