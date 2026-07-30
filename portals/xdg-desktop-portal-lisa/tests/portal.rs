//! The portal over zbus peer-to-peer connections (PLAN §5.5). P2p over
//! a socketpair needs no bus daemon, so the whole trust boundary —
//! consent, grants, quotas, Ledger attribution, revocation killing live
//! sessions — runs on macOS dev hosts and CI alike. The §5.5 acceptance
//! items that need a real desktop (Flatpak sandbox, consent dialog
//! pixels) are exercised on Linux systems.

use lisa_peer::app::{AppIdentity, StaticIdentity};
use lisa_portal::consent::{ConsentUi, StaticConsent};
use lisa_portal::grants::{Effective, GrantAction, GrantStore};
use lisa_portal::portal::{PORTAL_PATH, PortalState, serve_on_builder};
use lisa_portal::quota::QuotaConfig;
use lisa_portal::upstream::stub::StubUpstream;
use lisa_portal::upstream::{InferenceUpstream, ZbusUpstream};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

struct Harness {
    server: zbus::Connection,
    client: zbus::Connection,
    state: Arc<PortalState>,
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
    let state: Arc<PortalState> = PortalState::new(
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
        state,
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
        "dev.lisaos.Portal",
        PORTAL_PATH,
        "dev.lisaos.portal.Inference",
    )
    .await
    .unwrap()
}

async fn grants_proxy(h: &Harness) -> zbus::Proxy<'static> {
    zbus::Proxy::new(
        &h.client,
        "dev.lisaos.Portal",
        PORTAL_PATH,
        "dev.lisaos.portal.Grants",
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
        "dev.lisaos.Portal",
        path,
        "dev.lisaos.portal.Session",
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
    // Straight at the revocation, not through the D-Bus verb: writing a
    // grant action now requires being an allowlisted program on a real
    // broker (issue #107), which `tests/bus.rs` covers. What is under
    // test here is that a revoke kills the live session in time.
    h.grants
        .record("host:demo", "inference", GrantAction::Revoke)
        .unwrap();
    h.ledger
        .append(&lisa_ledger::Event {
            kind: "context.grant".into(),
            app_id: "host:demo".into(),
            status: "revoked".into(),
            detail: "scope=inference action=revoke via=settings".into(),
            ..Default::default()
        })
        .unwrap();
    let killed = lisa_portal::portal::revoke_sessions(
        &h.state,
        h.server.object_server(),
        "host:demo",
        "inference",
    )
    .await
    .unwrap();
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
            ..QuotaConfig::default()
        },
    )
    .await;
    // OpenSession spends one of the two (issue #111 — it used to be
    // free), so exactly one Embed is left in the window.
    let (path, _fd) = open_session(&h).await.unwrap();
    let session = session_proxy(&h, path).await;
    session.call_method("Embed", &(vec!["x"],)).await.unwrap();
    let err = session
        .call_method("Embed", &(vec!["x"],))
        .await
        .expect_err("the third request in the window must hit the quota");
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
            ..QuotaConfig::default()
        },
    )
    .await;
    let (path, _fd) = open_session(&h).await.unwrap();
    let session = session_proxy(&h, path).await;
    // Four words fit inside the 5-token budget.
    session
        .call_method("Embed", &(vec!["one two three four"],))
        .await
        .unwrap();
    let err = session
        .call_method("Embed", &(vec!["more words than are left"],))
        .await
        .expect_err("budget is spent");
    assert!(err.to_string().contains("LimitsExceeded"), "{err}");
}

/// Issue #114 through the real surface: a request bigger than the whole
/// day's budget is refused outright rather than admitted and charged.
/// Before the fix this call succeeded and left the counter at ~1000
/// against a cap of 5.
#[tokio::test]
async fn one_huge_request_cannot_blow_through_the_daily_budget() {
    let h = harness_with(
        AppIdentity::host("host:hog"),
        Arc::new(StaticConsent::allow_always()),
        Arc::new(StubUpstream),
        QuotaConfig {
            requests_per_min: 1000,
            tokens_per_day: 5,
            ..QuotaConfig::default()
        },
    )
    .await;
    let (path, _fd) = open_session(&h).await.unwrap();
    let session = session_proxy(&h, path).await;
    let huge = "word ".repeat(1000);
    let err = session
        .call_method("Embed", &(vec![huge],))
        .await
        .expect_err("a request larger than the whole budget must be refused");
    assert!(err.to_string().contains("LimitsExceeded"), "{err}");
    assert_eq!(
        h.grants
            .tokens_used("host:hog", &lisa_portal::quota::day_key(now_secs()))
            .unwrap(),
        0,
        "a refused request must not be charged"
    );
}

/// Issue #114's other half: output is charged for, so a two-word prompt
/// cannot drive an unbounded generation for two tokens.
#[tokio::test]
async fn a_generation_is_charged_for_the_output_it_reserves() {
    let h = harness_with(
        AppIdentity::host("host:writer"),
        Arc::new(StaticConsent::allow_always()),
        Arc::new(StubUpstream),
        QuotaConfig {
            requests_per_min: 1000,
            tokens_per_day: 100,
            assumed_output_tokens: 500,
            ..QuotaConfig::default()
        },
    )
    .await;
    let (path, _fd) = open_session(&h).await.unwrap();
    let session = session_proxy(&h, path).await;
    // Two words, but 500 reserved for what comes back: over budget.
    let err = session
        .call_method(
            "Generate",
            &("write everything", HashMap::<String, OwnedValue>::new()),
        )
        .await
        .expect_err("an unbounded generation must be charged for its output");
    assert!(err.to_string().contains("LimitsExceeded"), "{err}");

    // Stating a small ceiling brings it inside the budget.
    let mut params = HashMap::<String, OwnedValue>::new();
    params.insert(
        "max_tokens".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from(10i64)).unwrap(),
    );
    session
        .call_method("Generate", &("write everything", params))
        .await
        .expect("a bounded generation fits");
}

/// Issue #111: OpenSession was neither rate-limited nor capped, so an
/// app could hold unbounded upstream sessions, file descriptors and
/// D-Bus objects. Fifty in a row used to be admitted with the request
/// quota set to 1.
#[tokio::test]
async fn open_session_is_rate_limited_and_capped() {
    let h = harness_with(
        AppIdentity::host("host:leak"),
        Arc::new(StaticConsent::allow_always()),
        Arc::new(StubUpstream),
        QuotaConfig {
            requests_per_min: 1,
            tokens_per_day: 1_000_000,
            ..QuotaConfig::default()
        },
    )
    .await;
    open_session(&h).await.expect("the first one is allowed");
    let err = open_session(&h)
        .await
        .expect_err("opening sessions must spend the request quota");
    assert!(err.to_string().contains("LimitsExceeded"), "{err}");
}

/// The concurrent-session cap, separately from the rate limit: an app
/// that opens slowly still cannot hold an unbounded number open.
#[tokio::test]
async fn an_app_cannot_hold_unbounded_sessions_open() {
    let h = harness_with(
        AppIdentity::host("host:leak"),
        Arc::new(StaticConsent::allow_always()),
        Arc::new(StubUpstream),
        QuotaConfig {
            requests_per_min: 1000,
            tokens_per_day: 1_000_000,
            max_sessions_per_app: 3,
            ..QuotaConfig::default()
        },
    )
    .await;
    let mut open = Vec::new();
    for i in 0..3 {
        open.push(
            open_session(&h)
                .await
                .unwrap_or_else(|e| panic!("session {i}: {e}")),
        );
    }
    let err = open_session(&h)
        .await
        .expect_err("the fourth session must be refused");
    assert!(err.to_string().contains("LimitsExceeded"), "{err}");

    // Closing one makes room again — the cap counts what is open, not
    // what was ever opened.
    let (path, _) = open.pop().unwrap();
    session_proxy(&h, path)
        .await
        .call_method("Close", &())
        .await
        .unwrap();
    open_session(&h)
        .await
        .expect("a closed session frees its slot");
}

/// Issue #113: a refusal the user did not ask to remember left no trace,
/// so an app could re-summon the dialog forever and win by attrition.
/// The consent UI here says "no, not now" every time; it must stop being
/// asked.
#[tokio::test]
async fn an_app_cannot_re_prompt_until_the_user_gives_in() {
    #[derive(Default)]
    struct CountingConsent {
        asked: std::sync::atomic::AtomicUsize,
    }
    impl ConsentUi for CountingConsent {
        fn ask(
            &self,
            _app: &AppIdentity,
            _scope: &str,
        ) -> futures::future::BoxFuture<'_, Option<lisa_portal::consent::ConsentReply>> {
            self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move {
                Some(lisa_portal::consent::ConsentReply {
                    allow: false,
                    remember: false,
                })
            })
        }
    }

    let consent = Arc::new(CountingConsent::default());
    let h = harness_with(
        AppIdentity::host("host:nag"),
        Arc::clone(&consent) as Arc<dyn ConsentUi>,
        Arc::new(StubUpstream),
        QuotaConfig::default(),
    )
    .await;
    for _ in 0..25 {
        assert!(
            open_session(&h).await.is_err(),
            "a refused app got a session"
        );
    }
    let asked = consent.asked.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        asked <= lisa_portal::consent::PromptPolicy::default().max_refusals as usize,
        "the user was asked {asked} times after refusing"
    );
    // And the app is not permanently denied — the user said "not now",
    // not "never".
    assert_eq!(
        h.grants.effective("host:nag", "inference").unwrap(),
        Effective::Unset
    );
}

/// Issue #107. Grant management used to reject only Flatpak callers,
/// which meant every other process could mint a grant for any app id.
/// Now it takes an identified, allowlisted program — and a caller the
/// portal cannot identify at all is nobody, so all four verbs refuse.
///
/// This transport has no credentials to offer, which is precisely the
/// case that has to fail closed. The *allowed* path needs a real broker
/// and a real executable: `tests/bus.rs`.
#[tokio::test]
async fn grant_management_refuses_every_unidentified_caller() {
    let h = harness(
        AppIdentity::host("host:whoever"),
        Arc::new(StaticConsent::unavailable()),
    )
    .await;
    let grants = grants_proxy(&h).await;
    for (verb, body) in [
        ("Grant", ("org.example.Victim", "inference")),
        ("Deny", ("org.gnome.Calendar", "inference")),
    ] {
        let err = grants.call_method(verb, &body).await.unwrap_err();
        assert!(err.to_string().contains("AccessDenied"), "{verb}: {err}");
    }
    let err = grants
        .call_method("Revoke", &("org.example.Victim", "inference"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("AccessDenied"), "{err}");
    let err = grants.call_method("List", &()).await.unwrap_err();
    assert!(err.to_string().contains("AccessDenied"), "{err}");

    // Nothing was written: the victim has no grant it never consented to.
    assert_eq!(
        h.grants
            .effective("org.example.Victim", "inference")
            .unwrap(),
        Effective::Unset
    );
    assert_eq!(
        h.grants
            .effective("org.gnome.Calendar", "inference")
            .unwrap(),
        Effective::Unset,
        "a lock-out was persisted by an unauthenticated caller"
    );
}

#[tokio::test]
async fn a_pre_granted_app_opens_without_a_prompt() {
    let h = harness(
        AppIdentity::host("host:settings-demo"),
        // Consent backend absent: only the pre-grant can authorize.
        Arc::new(StaticConsent::unavailable()),
    )
    .await;
    // Seeded through the store rather than the D-Bus verb: writing a
    // grant now requires being an allowlisted program, and this test is
    // about what a pre-grant *does*, not about who may write one.
    h.grants
        .record("host:settings-demo", "inference", GrantAction::Allow)
        .unwrap();
    open_session(&h)
        .await
        .expect("pre-granted app opens with no prompt");

    let rows = h.grants.list().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].app_id, "host:settings-demo");
    assert_eq!(rows[0].state, Effective::Allowed);
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
    // End-to-end over two p2p hops: app ↔ portal ↔ dev.lisaos.Inference1
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
            "/dev/lisaos/Inference1",
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
