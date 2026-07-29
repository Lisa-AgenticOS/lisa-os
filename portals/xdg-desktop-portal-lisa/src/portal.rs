//! The portal D-Bus surface (`docs/PLAN.md` §5.5, ADR-0008):
//!
//! - `dev.lisaos.portal.Inference` at `/dev/lisaos/portal/desktop` —
//!   `OpenSession(a{sv}) → (o, h)`: identity → consent → Ledger →
//!   proxied `dev.lisaos.Inference1` session. The returned fd is the
//!   daemon's token pipe, passed through untouched; the returned object
//!   path is a portal-owned session (`dev.lisaos.portal.Session`) that
//!   proxies Generate/Embed/Cancel/Close with per-call Ledger
//!   attribution and quota enforcement.
//! - `dev.lisaos.portal.Grants` at the same path — the Settings ›
//!   Intelligence backend: List/Grant/Deny/Revoke. Revoke kills every
//!   live session under the grant: the daemon session is closed (the
//!   app's fd sees EOF) and the portal session object is removed, well
//!   under the 1 s acceptance budget.
//!
//! `dev.lisaos.portal.{Context,Memory,Agent}` (§5.5) are reserved names,
//! landing with M3/M5 on this same grant store.
//!
//! Tested over zbus p2p connections (no bus daemon needed — macOS dev
//! hosts and CI alike); session-bus registration happens on real systems.

use crate::SCOPE_INFERENCE;
use crate::consent::{Authorization, ConsentUi, PromptPolicy, authorize, needs_prompt};
use crate::grants::{GrantAction, GrantStore};
use crate::identity::{AppIdentity, IdentityResolver};
use crate::manager::{may_manage, resolve_managers};
use crate::quota::{QuotaBook, QuotaConfig, QuotaExceeded, day_key, estimate_tokens};
use crate::upstream::{InferenceUpstream, UpstreamSession};
use lisa_ledger::{Event as LedgerEvent, Ledger, preview_of};
use lisa_peer::{Owner, Peer, PeerId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub const PORTAL_BUS_NAME: &str = "dev.lisaos.Portal";
pub const PORTAL_PATH: &str = "/dev/lisaos/portal/desktop";

/// Everything the interfaces decide with, shared across objects.
pub struct PortalState {
    pub identity: Arc<dyn IdentityResolver>,
    pub consent: Arc<dyn ConsentUi>,
    pub upstream: Arc<dyn InferenceUpstream>,
    pub grants: Arc<GrantStore>,
    pub ledger: Arc<Ledger>,
    pub quota_cfg: QuotaConfig,
    pub prompt_policy: PromptPolicy,
    /// Programs allowed to write grants (issue #107). Configured paths,
    /// resolved freshly at each check — see [`resolve_managers`].
    pub managers: Vec<PathBuf>,
    quota: Mutex<QuotaBook>,
    sessions: Mutex<HashMap<String, LiveSession>>,
    next_session: AtomicU64,
    /// Per-process secret behind the session-path tokens. Never leaves
    /// the portal; see [`PortalState::mint_session_token`].
    path_secret: [u8; 32],
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
        Self::with_policy(
            identity,
            consent,
            upstream,
            grants,
            ledger,
            quota_cfg,
            PromptPolicy::default(),
            crate::manager::default_managers(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_policy(
        identity: Arc<dyn IdentityResolver>,
        consent: Arc<dyn ConsentUi>,
        upstream: Arc<dyn InferenceUpstream>,
        grants: Arc<GrantStore>,
        ledger: Arc<Ledger>,
        quota_cfg: QuotaConfig,
        prompt_policy: PromptPolicy,
        managers: Vec<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            identity,
            consent,
            upstream,
            grants,
            ledger,
            quota_cfg,
            prompt_policy,
            managers,
            quota: Mutex::new(QuotaBook::default()),
            sessions: Mutex::new(HashMap::new()),
            next_session: AtomicU64::new(1),
            path_secret: random_secret(),
        })
    }

    /// An unguessable session token (issue #108).
    ///
    /// Paths were `/dev/lisaos/portal/session/{1,2,3,…}`, so "the path
    /// is a capability" was never true — anyone could write down the
    /// scheme and count. The path is *still* not the capability (the
    /// ownership check is), but a guessable identifier is a free
    /// enumeration of everyone else's live sessions, and there is no
    /// reason to hand it over.
    ///
    /// Derived rather than drawn per session: one read from the OS at
    /// startup, keyed by a counter, so minting a path is arithmetic and
    /// cannot fail halfway through a request.
    fn mint_session_token(&self) -> String {
        let n = self.next_session.fetch_add(1, Ordering::Relaxed);
        let mut hasher = blake3::Hasher::new_keyed(&self.path_secret);
        hasher.update(&n.to_le_bytes());
        hasher.finalize().to_hex()[..32].to_string()
    }

    /// How many sessions this app currently holds open.
    fn open_sessions_for(&self, app_id: &str) -> usize {
        self.sessions
            .lock()
            .expect("session registry lock")
            .values()
            .filter(|s| s.app_id == app_id)
            .count()
    }

    /// Live sessions for (app, scope) — removed from the registry and
    /// returned so the caller can close them outside the lock.
    fn take_sessions(&self, app_id: &str, scope: &str) -> Vec<LiveSession> {
        let mut sessions = self.sessions.lock().expect("session registry lock");
        let ids: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.app_id == app_id && s.scope == scope)
            .map(|(id, _)| id.clone())
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

/// 32 bytes from the OS, for [`PortalState::mint_session_token`].
///
/// A portal that cannot read `/dev/urandom` is on a system too broken to
/// serve a trust boundary, so this panics rather than quietly falling
/// back to something predictable — a weak fallback here would be
/// invisible in every test and wrong on every real machine.
fn random_secret() -> [u8; 32] {
    use std::io::Read;
    let mut buf = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .expect("/dev/urandom is required to mint session paths");
    buf
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn store_err(e: crate::grants::GrantError) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}

/// Who is calling, decided by the transport (ADR-0033).
///
/// This replaced a local `peer_pid` helper that asked the bus for the
/// sender's pid and handed the bare number to the identity resolver. Two
/// things were wrong with it and both are now somebody else's job:
/// `lisa_peer::resolve` refuses to ask a p2p peer about itself (#133),
/// and it keeps the broker's pidfd so `/proc` reads cannot land on a
/// recycled pid (#136).
async fn caller(conn: &zbus::Connection, header: &zbus::message::Header<'_>) -> Option<Peer> {
    lisa_peer::resolve(conn, header).await.ok()
}

/// The error a caller gets for a session that is not theirs.
///
/// Deliberately identical to the one zbus produces for a path that does
/// not exist (ADR-0033 §4): a distinguishable refusal would let a sweep
/// map which sessions are live, which is the reconnaissance for the next
/// attempt. Being refused and not existing must look the same.
fn no_such_session(path: &str) -> zbus::fdo::Error {
    zbus::fdo::Error::UnknownObject(format!("Unknown object '{path}'"))
}

/// The `max_tokens` a Generate request states, if any.
///
/// D-Bus gives us whatever variant the app packed, so every integer
/// width is accepted rather than only the one our own SDK happens to
/// send — a caller whose binding chose `u32` must not silently fall back
/// to the (larger) default reservation.
fn max_tokens_of(params: &HashMap<String, OwnedValue>) -> Option<i64> {
    let v = params.get("max_tokens")?;
    i64::try_from(v)
        .or_else(|_| u32::try_from(v).map(i64::from))
        .or_else(|_| i32::try_from(v).map(i64::from))
        .or_else(|_| u64::try_from(v).map(|n| n as i64))
        .or_else(|_| u16::try_from(v).map(i64::from))
        .ok()
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
            // Issue #113: an app that may raise a dialog on every attempt
            // does not have to defeat consent, only outlast the person
            // answering it. After enough refusals in the window, asking
            // simply stops working.
            let refusals = state
                .grants
                .refusals_since(
                    &app.app_id,
                    scope,
                    state.prompt_policy.window_start(now_ms()),
                )
                .map_err(store_err)?;
            if !state.prompt_policy.may_prompt(refusals) {
                // Nothing recorded here on purpose: writing another
                // refusal would push the window forward on every attempt
                // and the cooldown would never end — a permanent denial
                // the user never asked for.
                let _ = state.ledger.append(&LedgerEvent {
                    kind: "context.grant".into(),
                    app_id: app.app_id.clone(),
                    status: "denied".into(),
                    detail: format!("scope={scope} reason=prompt-cooldown refusals={refusals}"),
                    ..Default::default()
                });
                return Err(zbus::fdo::Error::AccessDenied(format!(
                    "{} was refused `{scope}` several times just now — \
                     it cannot ask again yet",
                    app.app_id
                )));
            }
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
                            app.kind.as_str()
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
                    detail: format!("scope={scope} identity={}", app.kind.as_str()),
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

#[zbus::interface(name = "dev.lisaos.portal.Inference")]
impl InferencePortal {
    /// Liveness probe.
    fn ping(&self) -> String {
        format!("xdg-desktop-portal-lisa {}", env!("CARGO_PKG_VERSION"))
    }

    /// Open an inference session for the calling app. Options are
    /// forwarded to `dev.lisaos.Inference1.OpenSession` ("model_hint" et
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
        let Some(peer) = caller(conn, &header).await else {
            return Err(zbus::fdo::Error::AccessDenied(
                "the caller could not be identified — refusing".into(),
            ));
        };
        let app = state.identity.identify(&peer);

        // Issue #111: opening a session was free. Every one of these is
        // an upstream daemon session, a file descriptor and a registered
        // object, and 50 in a row were admitted with the request quota
        // set to 1 — the gate only ever guarded Generate and Embed. The
        // rate check comes before consent so a runaway loop cannot spend
        // the user's attention either.
        state
            .quota
            .lock()
            .expect("quota lock")
            .check_request(&app.app_id, &state.quota_cfg, now_secs())
            .map_err(|e| zbus::fdo::Error::LimitsExceeded(e.to_string()))?;
        if state.open_sessions_for(&app.app_id) >= state.quota_cfg.max_sessions_per_app {
            return Err(zbus::fdo::Error::LimitsExceeded(
                QuotaExceeded::Sessions.to_string(),
            ));
        }

        self.authorize_scope(&app, SCOPE_INFERENCE).await?;

        // No ledger entry, no session (PLAN §4 rule 4).
        state.ledger_gate(&LedgerEvent {
            kind: "inference.session".into(),
            app_id: app.app_id.clone(),
            status: "started".into(),
            detail: format!(
                "portal scope={SCOPE_INFERENCE} identity={}",
                app.kind.as_str()
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

        let id = state.mint_session_token();
        let path = OwnedObjectPath::try_from(format!("/dev/lisaos/portal/session/{id}"))
            .expect("session path is valid");
        let session = PortalSession {
            state: Arc::clone(&state),
            id: id.clone(),
            app_id: app.app_id.clone(),
            // Issue #108: the session belongs to the peer that opened
            // it, and every later call on it is checked against this.
            // The registry below deliberately does NOT keep a second
            // copy — one owner, one place, no chance of the two
            // disagreeing about who may drive a session.
            owner: Owner::of(peer.id.clone()),
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
    id: String,
    app_id: String,
    /// The peer that opened this session. Every method checks it before
    /// doing anything (issue #108): without it, any app on the shared
    /// `dev.lisaos.Portal` name could cancel a neighbour's generation,
    /// or run one billed to the neighbour's quota and written into the
    /// Ledger under the neighbour's name.
    owner: Owner,
    upstream: Arc<dyn UpstreamSession>,
}

impl PortalSession {
    /// The caller, if this session is theirs.
    ///
    /// Refuses with [`no_such_session`] either way — not being the owner
    /// and not existing are the same answer, on purpose.
    async fn require_owner(
        &self,
        conn: &zbus::Connection,
        header: &zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let path = header
            .path()
            .map(|p| p.to_string())
            .unwrap_or_else(|| "/".into());
        let Ok(id) = PeerId::of(conn, header) else {
            return Err(no_such_session(&path));
        };
        self.owner.require(&id).map_err(|_| no_such_session(&path))
    }

    /// Charge one request against the app's budgets.
    ///
    /// The rate window and the daily token budget, in that order, with
    /// the token half all-or-nothing (issue #114): the request either
    /// fits in what is left of the day or it does not happen. It used to
    /// read the counter, compare, and add in a separate step — so an
    /// oversized request was admitted whole and two concurrent ones both
    /// spent the same remaining budget.
    fn quota_gate(&self, tokens: i64) -> zbus::fdo::Result<()> {
        let state = &self.state;
        let now = now_secs();
        state
            .quota
            .lock()
            .expect("quota lock")
            .check_request(&self.app_id, &state.quota_cfg, now)
            .map_err(|e| zbus::fdo::Error::LimitsExceeded(e.to_string()))?;
        let admitted = state
            .grants
            .try_spend_tokens(
                &self.app_id,
                &day_key(now),
                tokens,
                state.quota_cfg.tokens_per_day,
            )
            .map_err(store_err)?;
        if !admitted {
            return Err(zbus::fdo::Error::LimitsExceeded(
                QuotaExceeded::Tokens.to_string(),
            ));
        }
        Ok(())
    }
}

#[zbus::interface(name = "dev.lisaos.portal.Session")]
impl PortalSession {
    /// Generate from `prompt`; tokens stream over the fd returned by
    /// OpenSession. Params are forwarded to the daemon session
    /// ("schema", "max_tokens", "priority").
    async fn generate(
        &self,
        prompt: String,
        params: HashMap<String, OwnedValue>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.require_owner(conn, &header).await?;
        self.quota_gate(crate::quota::generation_cost(
            &prompt,
            max_tokens_of(&params),
            &self.state.quota_cfg,
        ))?;
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
    ///
    /// Charged for its input only, and correctly so: an embedding's
    /// output is a fixed-size vector, not a generation, so there is
    /// nothing to reserve.
    async fn embed(
        &self,
        texts: Vec<String>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<Vec<Vec<f64>>> {
        self.require_owner(conn, &header).await?;
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
    async fn cancel(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.require_owner(conn, &header).await?;
        self.upstream
            .cancel()
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    /// Close the session: daemon side first, then the portal object.
    async fn close(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.require_owner(conn, &header).await?;
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
    /// never for apps — an app must not grant itself, or anybody else,
    /// a scope.
    ///
    /// This used to reject only Flatpak callers, which meant every
    /// unsandboxed process on the session bus could mint a grant for any
    /// app id, or write a remembered `Deny` and lock an app out for good
    /// (issue #107). The check is now the caller's *executable* against
    /// the shipped allowlist — see `crate::manager`.
    async fn require_manager(
        &self,
        conn: &zbus::Connection,
        header: &zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let refuse = |detail: String| {
            let _ = self.state.ledger.append(&LedgerEvent {
                kind: "context.grant".into(),
                app_id: "portal".into(),
                status: "denied".into(),
                detail,
                ..Default::default()
            });
            zbus::fdo::Error::AccessDenied(
                "only Settings and the lisa CLI can change grants".into(),
            )
        };
        let Some(peer) = caller(conn, header).await else {
            return Err(refuse("grant management: caller unidentified".into()));
        };
        #[cfg(unix)]
        let exe = lisa_peer::exe_of_peer(&peer).ok();
        #[cfg(not(unix))]
        let exe: Option<PathBuf> = None;
        let managers = resolve_managers(&self.state.managers);
        may_manage(peer.is_same_user_as_us(), exe.as_deref(), &managers).map_err(|why| {
            // The Ledger names the program that tried; the caller is
            // told only that it may not.
            refuse(format!(
                "grant management refused: {why} exe={}",
                exe.as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "?".into())
            ))
        })
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

#[zbus::interface(name = "dev.lisaos.portal.Grants")]
impl GrantsPortal {
    /// Every (app, scope) that ever asked: (app_id, scope, state) with
    /// state one of "allowed" | "denied" | "unset".
    ///
    /// Manager-only like the writes. It is read-only, but it is also a
    /// list of what the user has installed and what they have refused —
    /// reconnaissance for a caller deciding which name to impersonate
    /// next, and no ordinary app has business reading it.
    async fn list(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<Vec<(String, String, String)>> {
        self.require_manager(conn, &header).await?;
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
        self.require_manager(conn, &header).await?;
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
        self.require_manager(conn, &header).await?;
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
        self.require_manager(conn, &header).await?;
        self.record_action(&app_id, &scope, GrantAction::Revoke, "revoked")?;
        revoke_sessions(&self.state, server, &app_id, &scope).await
    }
}

/// Record-free revocation of live sessions: close the daemon side (the
/// app's fd sees EOF) and unregister the portal object.
///
/// Split out from the D-Bus verb so the §5.5 acceptance property — a
/// revoke kills live sessions, well inside a second — stays testable now
/// that reaching the verb requires being an allowlisted program on a
/// real broker. The authorization is tested separately; this is the
/// part that does the work.
pub async fn revoke_sessions(
    state: &Arc<PortalState>,
    server: &ObjectServer,
    app_id: &str,
    scope: &str,
) -> zbus::fdo::Result<u32> {
    let doomed = state.take_sessions(app_id, scope);
    let mut killed = 0u32;
    for session in doomed {
        let _ = session.upstream.close().await;
        let _ = server.remove::<PortalSession, _>(&session.path).await;
        killed += 1;
    }
    Ok(killed)
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
