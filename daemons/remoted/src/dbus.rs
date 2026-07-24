//! D-Bus management surface: org.lisa.Remote1 (ADR-0008 §1, ADR-0010 §4)
//! — the Settings app's plane: providers, credentials (write-only),
//! per-scope offload consent, and "Sign in with Claude / ChatGPT" OAuth.
//! Tested over zbus p2p (macOS + CI); registered on the bus on real
//! systems.

use crate::service::Broker;
use std::sync::Arc;
use zbus::object_server::SignalEmitter;

/// The object path the interface lives at (session bus + p2p).
pub const OBJECT_PATH: &str = "/org/lisa/Remote1";

pub struct Remote1 {
    broker: Arc<Broker>,
}

impl Remote1 {
    pub fn new(broker: Arc<Broker>) -> Self {
        Self { broker }
    }
}

fn fail(e: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}

#[zbus::interface(name = "org.lisa.Remote1")]
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
    fn add_provider(
        &self,
        id: String,
        display_name: String,
        base_url: String,
    ) -> zbus::fdo::Result<()> {
        self.broker
            .add_provider(&id, &display_name, &base_url)
            .map_err(fail)
    }

    fn remove_provider(&self, id: String) -> zbus::fdo::Result<()> {
        self.broker.remove_provider(&id).map_err(fail)
    }

    /// Store a credential. Write-only: no method returns key material.
    fn set_key(&self, id: String, key: String) -> zbus::fdo::Result<()> {
        self.broker.set_key(&id, &key).map_err(fail)
    }

    fn clear_key(&self, id: String) -> zbus::fdo::Result<()> {
        self.broker.clear_key(&id).map_err(fail)
    }

    /// Flip a per-scope "may offload" switch (default: nothing leaves).
    fn set_consent(&self, scope: String, allowed: bool) -> zbus::fdo::Result<()> {
        self.broker.set_consent(&scope, allowed).map_err(fail)
    }

    /// Begin "Sign in with …" for an OAuth-capable provider (`anthropic`
    /// or `openai`); returns the authorize URL for the panel to open in
    /// the browser. The broker binds a loopback callback server and does
    /// the token exchange when the browser redirects back; completion
    /// arrives asynchronously via the `LoginCompleted` signal. Fails for
    /// key-only providers.
    async fn begin_login(&self, provider_id: String) -> zbus::fdo::Result<String> {
        self.broker.begin_login(&provider_id).await.map_err(fail)
    }

    /// Forget a stored OAuth session (idempotent).
    fn logout(&self, provider_id: String) -> zbus::fdo::Result<()> {
        self.broker.logout(&provider_id).map_err(fail)
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
        .name("org.lisa.Remoted")?
        .serve_at(OBJECT_PATH, Remote1::new(Arc::clone(&broker)))?
        .build()
        .await?;
    spawn_login_signal(broker, conn.clone());
    Ok(conn)
}
