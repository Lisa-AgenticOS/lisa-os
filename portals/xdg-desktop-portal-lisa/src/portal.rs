//! The portal D-Bus surface (`docs/PLAN.md` §5.5, ADR-0008):
//!
//! - `org.lisa.portal.Inference` at `/org/lisa/portal/desktop` —
//!   `OpenSession(a{sv}) → (o, h)`: identity → consent → Ledger →
//!   proxied `org.lisa.Inference1` session. The returned fd is the
//!   daemon's token pipe, passed through untouched; the returned object
//!   path is a portal-owned session (`org.lisa.portal.Session`) that
//!   proxies Generate/Embed/Cancel/Close with per-call Ledger
//!   attribution and quota enforcement.
//! - `org.lisa.portal.Grants` at the same path — the Settings ›
//!   Intelligence backend: List/Grant/Deny/Revoke. Revoke kills every
//!   live session under the grant: the daemon session is closed (the
//!   app's fd sees EOF) and the portal session object is removed, well
//!   under the 1 s acceptance budget.
//!
//! `org.lisa.portal.{Context,Memory,Agent}` (§5.5) are reserved names,
//! landing with M3/M5 on this same grant store.
//!
//! Tested over zbus p2p connections (no bus daemon needed — macOS dev
//! hosts and CI alike); session-bus registration happens on real systems.

use crate::SCOPE_INFERENCE;
use crate::consent::{Authorization, ConsentUi, authorize, needs_prompt};
use crate::grants::{GrantAction, GrantStore};
use crate::identity::{AppIdentity, IdentityKind, IdentityResolver};
use crate::quota::{QuotaBook, QuotaConfig, check_tokens, day_key, estimate_tokens};
use crate::upstream::{InferenceUpstream, UpstreamSession};
use lisa_ledger::{Event as LedgerEvent, Ledger, preview_of};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub const PORTAL_BUS_NAME: &str = "org.lisa.Portal";
pub const PORTAL_PATH: &str = "/org/lisa/portal/desktop";

/// Everything the interfaces decide with, shared across objects.
pub struct PortalState {
    pub identity: Arc<dyn IdentityResolver>,
    pub consent: Arc<dyn ConsentUi>,
    pub upstream: Arc<dyn InferenceUpstream>,
    pub grants: Arc<GrantStore>,
    pub ledger: Arc<Ledger>,
    pub quota_cfg: QuotaConfig,
    quota: Mutex<QuotaBook>,
    sessions: Mutex<HashMap<u64, LiveSession>>,
    next_session: AtomicU64,
}

struct LiveSession {
    app_id: String,
    scope: String,
    path: OwnedObjectPath,
    upstream: Arc<dyn UpstreamSession>,
}

impl PortalState {
    pub fn new(
        identity: Arc<dyn IdentityResolver>,
        consent: Arc<dyn ConsentUi>,
        upstream: Arc<dyn InferenceUpstream>,
        grants: Arc<GrantStore>,
        ledger: Arc<Ledger>,
        quota_cfg: QuotaConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            identity,
            consent,
            upstream,
            grants,
            ledger,
            quota_cfg,
            quota: Mutex::new(QuotaBook::default()),
            sessions: Mutex::new(HashMap::new()),
            next_session: AtomicU64::new(1),
        })
    }

    /// Live sessions for (app, scope) — removed from the registry and
    /// returned so the caller can close them outside the lock.
    fn take_sessions(&self, app_id: &str, scope: &str) -> Vec<LiveSession> {
        let mut sessions = self.sessions.lock().expect("session registry lock");
        let ids: Vec<u64> = sessions
            .iter()
            .filter(|(_, s)| s.app_id == app_id && s.scope == scope)
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter()
            .filter_map(|id| sessions.remove(&id))
            .collect()
    }

    /// Dataflow rule 4 (PLAN §4): the ledger entry precedes the action.
    fn ledger_gate(&self, event: &LedgerEvent) -> zbus::fdo::Result<i64> {
        self.ledger.append(event).map_err(|e| {
            zbus::fdo::Error::Failed(format!("refusing to act without a ledger entry: {e}"))
        })
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn store_err(e: crate::grants::GrantError) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}

/// Resolve the caller's pid via the bus daemon's credentials (absent on
/// p2p transports — the identity resolver decides what that means).
async fn peer_pid(conn: &zbus::Connection, header: &zbus::message::Header<'_>) -> Option<u32> {
    let sender = header.sender()?.to_owned();
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let creds = dbus.get_connection_credentials(sender.into()).await.ok()?;
    creds.process_id()
}

fn identity_kind_str(kind: &IdentityKind) -> &'static str {
    match kind {
        IdentityKind::Flatpak => "flatpak",
        IdentityKind::Host => "host",
    }
}

pub struct InferencePortal {
    state: Arc<PortalState>,
}

impl InferencePortal {
    pub fn new(state: Arc<PortalState>) -> Self {
        Self { state }
    }

    /// Consent + grant bookkeeping for one request. Fail-closed: any
    /// bookkeeping failure on the granted path refuses the session.
    async fn authorize_scope(&self, app: &AppIdentity, scope: &str) -> zbus::fdo::Result<()> {
        let state = &self.state;
        let effective = state
            .grants
            .effective(&app.app_id, scope)
            .map_err(store_err)?;
        let reply = if needs_prompt(effective) {
            state.consent.ask(app, scope).await
        } else {
            None
        };
        match authorize(effective, reply) {
            Authorization::Granted { record } => {
                if let Some(action) = record {
                    state
                        .grants
                        .record(&app.app_id, scope, action)
                        .map_err(store_err)?;
                    state.ledger_gate(&LedgerEvent {
                        kind: "context.grant".into(),
                        app_id: app.app_id.clone(),
                        status: "allowed".into(),
                        detail: format!(
                            "scope={scope} action={} identity={}",
                            action.as_str(),
                            identity_kind_str(&app.kind)
                        ),
                        ..Default::default()
                    })?;
                }
                Ok(())
            }
            Authorization::Denied { record } => {
                if let Some(action) = record {
                    state
                        .grants
                        .record(&app.app_id, scope, action)
                        .map_err(store_err)?;
                }
                let _ = state.ledger.append(&LedgerEvent {
                    kind: "context.grant".into(),
                    app_id: app.app_id.clone(),
                    status: "denied".into(),
                    detail: format!("scope={scope} identity={}", identity_kind_str(&app.kind)),
                    ..Default::default()
                });
                Err(zbus::fdo::Error::AccessDenied(format!(
                    "{} has no `{scope}` grant",
                    app.app_id
                )))
            }
        }
    }
}

#[zbus::interface(name = "org.lisa.portal.Inference")]
impl InferencePortal {
    /// Liveness probe.
    fn ping(&self) -> String {
        format!("xdg-desktop-portal-lisa {}", env!("CARGO_PKG_VERSION"))
    }

    /// Open an inference session for the calling app. Options are
    /// forwarded to `org.lisa.Inference1.OpenSession` ("model_hint" et
    /// al.); the portal adds "app_id". Returns the portal session object
    /// path and the daemon's token-pipe read fd.
    async fn open_session(
        &self,
        mut options: HashMap<String, OwnedValue>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<(OwnedObjectPath, zbus::zvariant::OwnedFd)> {
        let state = Arc::clone(&self.state);
        let pid = peer_pid(conn, &header).await;
        let app = state.identity.identify(pid);
        self.authorize_scope(&app, SCOPE_INFERENCE).await?;

        // No ledger entry, no session (PLAN §4 rule 4).
        state.ledger_gate(&LedgerEvent {
            kind: "inference.session".into(),
            app_id: app.app_id.clone(),
            status: "started".into(),
            detail: format!(
                "portal scope={SCOPE_INFERENCE} identity={}",
                identity_kind_str(&app.kind)
            ),
            ..Default::default()
        })?;

        options.insert(
            "app_id".into(),
            OwnedValue::try_from(Value::from(app.app_id.clone()))
                .expect("string converts to OwnedValue"),
        );
        let (upstream_session, fd) = state
            .upstream
            .open_session(options)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let upstream_session: Arc<dyn UpstreamSession> = Arc::from(upstream_session);

        let id = state.next_session.fetch_add(1, Ordering::Relaxed);
        let path = OwnedObjectPath::try_from(format!("/org/lisa/portal/session/{id}"))
            .expect("session path is valid");
        let session = PortalSession {
            state: Arc::clone(&state),
            id,
            app_id: app.app_id.clone(),
            upstream: Arc::clone(&upstream_session),
        };
        server
            .at(&path, session)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("registering session: {e}")))?;
        state
            .sessions
            .lock()
            .expect("session registry lock")
            .insert(
                id,
                LiveSession {
                    app_id: app.app_id,
                    scope: SCOPE_INFERENCE.into(),
                    path: path.clone(),
                    upstream: upstream_session,
                },
            );
        Ok((path, fd.into()))
    }
}

pub struct PortalSession {
    state: Arc<PortalState>,
    id: u64,
    app_id: String,
    upstream: Arc<dyn UpstreamSession>,
}

impl PortalSession {
    /// Quota gate for one request: sliding requests/min plus the
    /// persisted tokens/day budget, then account `tokens`.
    fn quota_gate(&self, tokens: i64) -> zbus::fdo::Result<()> {
        let state = &self.state;
        let now = now_secs();
        state
            .quota
            .lock()
            .expect("quota lock")
            .check_request(&self.app_id, &state.quota_cfg, now)
            .map_err(|e| zbus::fdo::Error::LimitsExceeded(e.to_string()))?;
        let day = day_key(now);
        let used = state
            .grants
            .tokens_used(&self.app_id, &day)
            .map_err(store_err)?;
        check_tokens(used, &state.quota_cfg)
            .map_err(|e| zbus::fdo::Error::LimitsExceeded(e.to_string()))?;
        state
            .grants
            .add_tokens(&self.app_id, &day, tokens)
            .map_err(store_err)?;
        Ok(())
    }
}

#[zbus::interface(name = "org.lisa.portal.Session")]
impl PortalSession {
    /// Generate from `prompt`; tokens stream over the fd returned by
    /// OpenSession. Params are forwarded to the daemon session
    /// ("schema", "max_tokens", "priority").
    async fn generate(
        &self,
        prompt: String,
        params: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<()> {
        self.quota_gate(estimate_tokens(&prompt))?;
        self.state.ledger_gate(&LedgerEvent {
            kind: "inference.generate".into(),
            app_id: self.app_id.clone(),
            input_hash: blake3::hash(prompt.as_bytes()).to_hex().to_string(),
            preview: preview_of(&prompt),
            status: "started".into(),
            detail: "portal".into(),
            ..Default::default()
        })?;
        self.upstream
            .generate(prompt, params)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Embed texts (aad), proxied with attribution and quota accounting.
    async fn embed(&self, texts: Vec<String>) -> zbus::fdo::Result<Vec<Vec<f64>>> {
        let joined = texts.join("\n");
        self.quota_gate(estimate_tokens(&joined))?;
        self.state.ledger_gate(&LedgerEvent {
            kind: "inference.embed".into(),
            app_id: self.app_id.clone(),
            input_hash: blake3::hash(joined.as_bytes()).to_hex().to_string(),
            preview: preview_of(&texts.join(" | ")),
            status: "started".into(),
            detail: "portal".into(),
            ..Default::default()
        })?;
        self.upstream
            .embed(texts)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Abort the in-flight generation (the fd sees early EOF).
    async fn cancel(&self) -> zbus::fdo::Result<()> {
        self.upstream
            .cancel()
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Close the session: daemon side first, then the portal object.
    async fn close(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.state
            .sessions
            .lock()
            .expect("session registry lock")
            .remove(&self.id);
        let _ = self.upstream.close().await;
        if let Some(path) = header.path() {
            let _ = server.remove::<PortalSession, _>(path).await;
        }
        Ok(())
    }
}

pub struct GrantsPortal {
    state: Arc<PortalState>,
}

impl GrantsPortal {
    pub fn new(state: Arc<PortalState>) -> Self {
        Self { state }
    }

    /// Grant management is for the user's own tooling (Settings, `lisa`),
    /// never for sandboxed apps — an app must not grant itself scopes.
    async fn require_host_caller(
        &self,
        conn: &zbus::Connection,
        header: &zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let app = self.state.identity.identify(peer_pid(conn, header).await);
        if app.kind == IdentityKind::Flatpak {
            return Err(zbus::fdo::Error::AccessDenied(
                "sandboxed apps cannot manage grants".into(),
            ));
        }
        Ok(())
    }

    fn record_action(
        &self,
        app_id: &str,
        scope: &str,
        action: GrantAction,
        status: &str,
    ) -> zbus::fdo::Result<()> {
        self.state
            .grants
            .record(app_id, scope, action)
            .map_err(store_err)?;
        self.state.ledger_gate(&LedgerEvent {
            kind: "context.grant".into(),
            app_id: app_id.into(),
            status: status.into(),
            detail: format!("scope={scope} action={} via=settings", action.as_str()),
            ..Default::default()
        })?;
        Ok(())
    }
}

#[zbus::interface(name = "org.lisa.portal.Grants")]
impl GrantsPortal {
    /// Every (app, scope) that ever asked: (app_id, scope, state) with
    /// state one of "allowed" | "denied" | "unset".
    async fn list(&self) -> zbus::fdo::Result<Vec<(String, String, String)>> {
        Ok(self
            .state
            .grants
            .list()
            .map_err(store_err)?
            .into_iter()
            .map(|row| (row.app_id, row.scope, row.state.as_str().to_string()))
            .collect())
    }

    /// Pre-grant a scope (Settings toggle on).
    async fn grant(
        &self,
        app_id: String,
        scope: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.require_host_caller(conn, &header).await?;
        self.record_action(&app_id, &scope, GrantAction::Allow, "allowed")
    }

    /// Persistently deny a scope (Settings toggle off, remembered).
    async fn deny(
        &self,
        app_id: String,
        scope: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.require_host_caller(conn, &header).await?;
        self.record_action(&app_id, &scope, GrantAction::Deny, "denied")
    }

    /// Revoke a grant and kill its live sessions (< 1 s, §5.5
    /// acceptance): the daemon session closes (the app's fd sees EOF)
    /// and the portal session object disappears. Returns the number of
    /// sessions killed. The next request prompts again.
    async fn revoke(
        &self,
        app_id: String,
        scope: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<u32> {
        self.require_host_caller(conn, &header).await?;
        self.record_action(&app_id, &scope, GrantAction::Revoke, "revoked")?;
        let doomed = self.state.take_sessions(&app_id, &scope);
        let mut killed = 0u32;
        for session in doomed {
            let _ = session.upstream.close().await;
            let _ = server.remove::<PortalSession, _>(&session.path).await;
            killed += 1;
        }
        Ok(killed)
    }
}

/// Register both interfaces on the session bus (real systems; tests use
/// p2p connections via [`serve_on_builder`]).
pub async fn serve(state: Arc<PortalState>) -> zbus::Result<zbus::Connection> {
    let builder = zbus::connection::Builder::session()?.name(PORTAL_BUS_NAME)?;
    serve_on_builder(builder, state)?.build().await
}

/// Attach the portal objects to any connection builder (session bus or
/// p2p test transports — bus-name claiming stays the caller's business).
pub fn serve_on_builder<'a>(
    builder: zbus::connection::Builder<'a>,
    state: Arc<PortalState>,
) -> zbus::Result<zbus::connection::Builder<'a>> {
    builder
        .serve_at(PORTAL_PATH, InferencePortal::new(Arc::clone(&state)))?
        .serve_at(PORTAL_PATH, GrantsPortal::new(state))
}
