//! D-Bus management surface: dev.lisaos.Remote1 (ADR-0008 §1, ADR-0010 §4)
//! — the Settings app's plane: providers, credentials (write-only),
//! per-scope offload consent, and "Sign in with Claude / ChatGPT" OAuth.
//! Tested over zbus p2p (macOS + CI); registered on the bus on real
//! systems.

use crate::service::Broker;
use lisa_peer::manager::Manager;
use std::path::PathBuf;
use std::sync::Arc;
use zbus::object_server::SignalEmitter;

/// The object path the interface lives at (session bus + p2p).
pub const OBJECT_PATH: &str = "/dev/lisaos/Remote1";

pub struct Remote1 {
    broker: Arc<Broker>,
    /// Programs allowed to operate this plane (issue #99).
    managers: Vec<PathBuf>,
}

impl Remote1 {
    pub fn new(broker: Arc<Broker>) -> Self {
        Self::with_managers(broker, lisa_peer::manager::default_managers())
    }

    pub fn with_managers(broker: Arc<Broker>, managers: Vec<PathBuf>) -> Self {
        Self { broker, managers }
    }

    /// Who is calling, and may they change anything?
    ///
    /// `dev.lisaos.Remote1` sits on the **session bus**, which anything
    /// the user runs can reach: a Flatpak app with session access, an
    /// app the agent built, a script. Before this there was no check of
    /// any kind, so all six offload scopes could be turned on by any of
    /// them with no prompt and no interaction — and the only trace was a
    /// `remote.consent` row attributed to Settings (#99).
    ///
    /// Reads stay open; this guards the mutations.
    async fn manager(
        &self,
        conn: &zbus::Connection,
        header: &zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<Manager> {
        let peer = lisa_peer::resolve(conn, header)
            .await
            .map_err(|_| refused(None))?;
        #[cfg(unix)]
        let exe = lisa_peer::exe_of_peer(&peer).ok();
        #[cfg(not(unix))]
        let exe: Option<PathBuf> = None;
        let managers = lisa_peer::manager::resolve_managers(&self.managers);
        Manager::authorize(peer.is_same_user_as_us(), exe.as_deref(), &managers)
            .map_err(|_| refused(exe.as_deref()))
    }
}

/// One refusal for every reason, so a caller cannot tell "you are not a
/// manager" from "we could not identify you" and probe accordingly. The
/// specific reason goes to the daemon's log, where the person who owns
/// the machine can read it.
fn refused(exe: Option<&std::path::Path>) -> zbus::fdo::Error {
    tracing::warn!(
        exe = exe.map(|p| p.display().to_string()).unwrap_or_default(),
        "refused a management call from a program that is not a manager"
    );
    zbus::fdo::Error::AccessDenied(
        "only Settings and the lisa CLI can change remote-provider settings".into(),
    )
}

fn fail(e: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}

#[zbus::interface(name = "dev.lisaos.Remote1")]
impl Remote1 {
    /// Liveness probe.
    fn ping(&self) -> String {
        format!("lisa-remoted {}", env!("CARGO_PKG_VERSION"))
    }

    /// Providers + credential presence + consent, one JSON document —
    /// the Settings page renders straight from this.
    fn state(&self) -> String {
        let mut v = self.broker.providers_json();
        v["may_offload"] = self.broker.consent_json()["may_offload"].clone();
        v.to_string()
    }

    /// Register a user-supplied OpenAI-compatible endpoint (§5.11).
    ///
    /// Public-internet rules only. An endpoint on this machine or this
    /// LAN is a deliberate, explained choice (#92), and Settings has no
    /// UI for that question — so it is made where the question can
    /// actually be asked: `lisa remote add --allow-local`.
    async fn add_provider(
        &self,
        id: String,
        display_name: String,
        base_url: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let who = self.manager(conn, &header).await?;
        self.broker
            .add_provider(
                &who,
                &id,
                &display_name,
                &base_url,
                crate::net::Locality::PublicOnly,
            )
            .map_err(fail)
    }

    async fn remove_provider(
        &self,
        id: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let who = self.manager(conn, &header).await?;
        self.broker.remove_provider(&who, &id).map_err(fail)
    }

    /// Store a credential. Write-only: no method returns key material.
    async fn set_key(
        &self,
        id: String,
        key: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let who = self.manager(conn, &header).await?;
        self.broker.set_key(&who, &id, &key).map_err(fail)
    }

    async fn clear_key(
        &self,
        id: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let who = self.manager(conn, &header).await?;
        self.broker.clear_key(&who, &id).map_err(fail)
    }

    /// Flip a per-scope "may offload" switch (default: nothing leaves).
    async fn set_consent(
        &self,
        scope: String,
        allowed: bool,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let who = self.manager(conn, &header).await?;
        self.broker.set_consent(&who, &scope, allowed).map_err(fail)
    }

    /// Begin "Sign in with …" for an OAuth-capable provider (`anthropic`
    /// or `openai`); returns the authorize URL for the panel to open in
    /// the browser. The broker binds a loopback callback server and does
    /// the token exchange when the browser redirects back; completion
    /// arrives asynchronously via the `LoginCompleted` signal. Fails for
    /// key-only providers.
    async fn begin_login(
        &self,
        provider_id: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<String> {
        let who = self.manager(conn, &header).await?;
        self.broker
            .begin_login(&who, &provider_id)
            .await
            .map_err(fail)
    }

    /// Forget a stored OAuth session (idempotent).
    async fn logout(
        &self,
        provider_id: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let who = self.manager(conn, &header).await?;
        self.broker.logout(&who, &provider_id).map_err(fail)
    }

    /// The provider's live model list (its own `/models`), as a JSON array
    /// of ids — for the Settings model dropdown. Requires a stored key.
    async fn list_models(&self, provider: String) -> zbus::fdo::Result<String> {
        let ids = self.broker.list_models(&provider).await.map_err(fail)?;
        Ok(serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_string()))
    }

    /// Emitted once a `BeginLogin` flow finishes: `ok` true on a stored
    /// session, false on error/timeout; `detail` is a human-readable
    /// status. No token material is ever carried.
    #[zbus(signal)]
    async fn login_completed(
        emitter: &SignalEmitter<'_>,
        provider_id: &str,
        ok: bool,
        detail: &str,
    ) -> zbus::Result<()>;
}

/// Forward broker login completions to the D-Bus `LoginCompleted` signal
/// on `conn`, and ledger successful sign-ins as egress-capability grants.
/// Runs until the broadcast sender is dropped.
pub fn spawn_login_signal(broker: Arc<Broker>, conn: zbus::Connection) {
    let mut rx = broker.subscribe_logins();
    tokio::spawn(async move {
        while let Ok(outcome) = rx.recv().await {
            if outcome.ok {
                broker.ledger_login_grant(&outcome.provider_id);
            }
            if let Ok(emitter) = SignalEmitter::new(&conn, OBJECT_PATH) {
                let _ = Remote1::login_completed(
                    &emitter,
                    &outcome.provider_id,
                    outcome.ok,
                    &outcome.detail,
                )
                .await;
            }
        }
    });
}

/// Register on the session bus (real systems; tests use p2p connections).
pub async fn serve(broker: Arc<Broker>) -> zbus::Result<zbus::Connection> {
    let conn = zbus::connection::Builder::session()?
        .name("dev.lisaos.Remoted")?
        .serve_at(OBJECT_PATH, Remote1::new(Arc::clone(&broker)))?
        .build()
        .await?;
    spawn_login_signal(broker, conn.clone());
    Ok(conn)
}
