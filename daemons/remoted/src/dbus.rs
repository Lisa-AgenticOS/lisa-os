//! D-Bus management surface: org.lisa.Remote1 (ADR-0008 §1) — the
//! Settings app's plane: providers, credentials (write-only), per-scope
//! offload consent, and the Sign in with Claude flow. Tested over zbus
//! p2p (macOS + CI); registered on the bus on real systems.

use crate::service::Broker;
use std::sync::Arc;

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

    /// Begin Sign in with Claude; returns the authorize URL to open.
    /// Fails with an explanatory error while endpoints are unset
    /// (ADR-0008 §4 — no invented URLs).
    fn claude_oauth_start(&self) -> zbus::fdo::Result<String> {
        self.broker.oauth_start().map_err(fail)
    }

    /// Complete the flow with the pasted authorization code.
    async fn claude_oauth_finish(&self, code: String) -> zbus::fdo::Result<()> {
        self.broker.oauth_finish(&code).await.map_err(fail)
    }
}

/// Register on the session bus (real systems; tests use p2p connections).
pub async fn serve(broker: Arc<Broker>) -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::session()?
        .name("org.lisa.Remoted")?
        .serve_at("/org/lisa/Remote1", Remote1::new(broker))?
        .build()
        .await
}
