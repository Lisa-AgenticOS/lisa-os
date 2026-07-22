//! The broker core shared by the unix-socket HTTP surface (`api.rs`)
//! and the D-Bus management surface (`dbus.rs`). All policy lives here:
//! registry, credentials, consent, OAuth state, and — load-bearing —
//! the Ledger gate: every remote request is written to the Ledger
//! *before* egress (dataflow rule 4), with the `remote.` kind prefix as
//! the machine-readable "leaves your hardware" marking (§5.11).

use crate::consent::{Consent, ConsentError};
use crate::oauth::{self, AuthorizeRequest, OauthEndpoints, OauthError, Pkce};
use crate::proxy::{self, ProxyError};
use crate::registry::{AuthStyle, Registry, RegistryError};
use crate::secrets::{SecretStore, SecretsError};
use lisa_ledger::{Event, Ledger, preview_of};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Secrets(#[from] SecretsError),
    #[error(transparent)]
    Consent(#[from] ConsentError),
    #[error(transparent)]
    Oauth(#[from] OauthError),
    #[error(transparent)]
    Proxy(#[from] ProxyError),
    #[error("refusing to run without a ledger entry: {0}")]
    Ledger(#[from] lisa_ledger::LedgerError),
    #[error("no Sign in with Claude flow in progress")]
    NoPendingOauth,
}

pub struct Broker {
    registry: Mutex<Registry>,
    secrets: SecretStore,
    consent: Mutex<Consent>,
    oauth_endpoints: OauthEndpoints,
    pending_oauth: Mutex<Option<AuthorizeRequest>>,
    ledger: Arc<Ledger>,
    http: reqwest::Client,
}

impl Broker {
    pub fn open(state_dir: &Path, ledger: Arc<Ledger>) -> anyhow::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            registry: Mutex::new(Registry::open(state_dir)?),
            secrets: SecretStore::open(state_dir)?,
            consent: Mutex::new(Consent::open(state_dir)?),
            oauth_endpoints: OauthEndpoints::load(state_dir)?,
            pending_oauth: Mutex::new(None),
            ledger,
            http: reqwest::Client::new(),
        }))
    }

    pub fn secrets(&self) -> &SecretStore {
        &self.secrets
    }

    pub fn with_registry<T>(&self, f: impl FnOnce(&Registry) -> T) -> T {
        f(&self.registry.lock().expect("registry lock"))
    }

    // ---- management plane -------------------------------------------------

    /// Providers with credential presence (never values) and OAuth
    /// availability, as one JSON document for UI surfaces.
    pub fn providers_json(&self) -> Value {
        let registry = self.registry.lock().expect("registry lock");
        let providers: Vec<Value> = registry
            .list()
            .into_iter()
            .map(|p| {
                let has_oauth_token = self.secrets.has_named(&format!("{}.oauth.json", p.id));
                json!({
                    "id": p.id,
                    "display_name": p.display_name,
                    "base_url": p.base_url,
                    "auth": p.auth,
                    "dialect": p.dialect,
                    "notes": p.notes,
                    "builtin": p.builtin,
                    "has_credential": self.secrets.has(&p.id) || has_oauth_token,
                    "oauth_available":
                        p.id == "anthropic" && self.oauth_endpoints.configured(),
                })
            })
            .collect();
        json!({ "providers": providers })
    }

    pub fn add_provider(&self, id: &str, name: &str, base_url: &str) -> Result<(), BrokerError> {
        Ok(self
            .registry
            .lock()
            .expect("registry lock")
            .add_custom(id, name, base_url)?)
    }

    pub fn remove_provider(&self, id: &str) -> Result<(), BrokerError> {
        self.registry
            .lock()
            .expect("registry lock")
            .remove_custom(id)?;
        // A removed provider's credential must not linger.
        let _ = self.secrets.remove(id);
        Ok(())
    }

    pub fn set_key(&self, id: &str, key: &str) -> Result<(), BrokerError> {
        // Only registered providers can hold credentials.
        self.registry.lock().expect("registry lock").get(id)?;
        Ok(self.secrets.set(id, key)?)
    }

    pub fn clear_key(&self, id: &str) -> Result<(), BrokerError> {
        Ok(self.secrets.remove(id)?)
    }

    pub fn consent_json(&self) -> Value {
        json!({ "may_offload": self.consent.lock().expect("consent lock").snapshot() })
    }

    pub fn set_consent(&self, scope: &str, allowed: bool) -> Result<(), BrokerError> {
        self.consent
            .lock()
            .expect("consent lock")
            .set(scope, allowed)?;
        // Consent flips are themselves auditable events.
        self.ledger.append(&Event {
            kind: "remote.consent".into(),
            app_id: "settings".into(),
            model: String::new(),
            input_hash: String::new(),
            preview: format!("may_offload {scope} = {allowed}"),
            status: "ok".into(),
            detail: json!({"egress": "remote", "scope": scope, "allowed": allowed}).to_string(),
            ..Default::default()
        })?;
        Ok(())
    }

    // ---- Sign in with Claude ----------------------------------------------

    /// Start the PKCE flow. Errors with `Unconfigured` until Anthropic
    /// publishes registerable endpoints (ADR-0008 §4 / rule 8).
    pub fn oauth_start(&self) -> Result<String, BrokerError> {
        let req = oauth::authorize_request(&self.oauth_endpoints, &Pkce::generate())?;
        let url = req.authorize_url.clone();
        *self.pending_oauth.lock().expect("oauth lock") = Some(req);
        Ok(url)
    }

    /// Finish the flow with the pasted authorization code: exchange it
    /// at the configured token endpoint and store the token set 0600.
    pub async fn oauth_finish(&self, code: &str) -> Result<(), BrokerError> {
        let pending = self
            .pending_oauth
            .lock()
            .expect("oauth lock")
            .take()
            .ok_or(BrokerError::NoPendingOauth)?;
        let form = oauth::token_exchange_form(&self.oauth_endpoints, code, &pending.verifier)?;
        let token_url = self
            .oauth_endpoints
            .token_url
            .clone()
            .ok_or(OauthError::Unconfigured)?;
        let resp = self
            .http
            .post(&token_url)
            .form(&form)
            .send()
            .await
            .map_err(ProxyError::from)?;
        let status = resp.status();
        let body: Value = resp.json().await.map_err(ProxyError::from)?;
        if !status.is_success() {
            return Err(ProxyError::Upstream {
                status: status.as_u16(),
                body: body.to_string(),
            }
            .into());
        }
        self.secrets
            .set_named("anthropic.oauth.json", &body.to_string())?;
        Ok(())
    }

    // ---- data plane --------------------------------------------------------

    fn credential_for(
        &self,
        id: &str,
        auth: AuthStyle,
    ) -> Result<(String, AuthStyle), BrokerError> {
        match auth {
            AuthStyle::Bearer => Ok((self.secrets.get(id)?, AuthStyle::Bearer)),
            // Anthropic: an API key wins; otherwise a stored Sign in
            // with Claude token authenticates as OAuth bearer.
            AuthStyle::AnthropicApiKey | AuthStyle::AnthropicOauth => {
                if let Ok(key) = self.secrets.get(id) {
                    return Ok((key, AuthStyle::AnthropicApiKey));
                }
                let raw = self.secrets.get_named(&format!("{id}.oauth.json"))?;
                let token = serde_json::from_str::<Value>(&raw)
                    .ok()
                    .and_then(|v| v["access_token"].as_str().map(String::from))
                    .ok_or_else(|| SecretsError::Missing(id.to_string()))?;
                Ok((token, AuthStyle::AnthropicOauth))
            }
        }
    }

    /// Proxy one chat completion. Ledger discipline (§5.11, dataflow
    /// rule 4): the `remote.generate` entry precedes egress — no entry,
    /// no request — and consent denials are ledgered refusals.
    pub async fn chat(
        &self,
        provider_id: &str,
        scopes: &[String],
        body: &Value,
    ) -> Result<Value, BrokerError> {
        let spec = self
            .registry
            .lock()
            .expect("registry lock")
            .get(provider_id)?;
        let model = body["model"].as_str().unwrap_or("").to_string();
        let ledger_model = format!("{}:{}", spec.id, model);
        let prompt_preview = body["messages"]
            .as_array()
            .and_then(|m| m.last())
            .and_then(|m| m["content"].as_str())
            .unwrap_or("")
            .to_string();
        let input_hash = blake3::hash(body.to_string().as_bytes())
            .to_hex()
            .to_string();
        let detail = json!({
            "egress": "remote",
            "provider": spec.id,
            "endpoint": spec.base_url,
            "scopes": scopes,
        })
        .to_string();

        if let Err(denied) = self.consent.lock().expect("consent lock").check(scopes) {
            self.ledger.append(&Event {
                kind: "remote.generate".into(),
                app_id: "host".into(),
                model: ledger_model,
                input_hash,
                preview: preview_of(&prompt_preview),
                status: "denied".into(),
                detail: json!({
                    "egress": "remote",
                    "provider": spec.id,
                    "scopes": scopes,
                    "reason": denied.to_string(),
                })
                .to_string(),
                ..Default::default()
            })?;
            return Err(denied.into());
        }

        let (credential, auth) = self.credential_for(&spec.id, spec.auth)?;
        let mut spec = spec;
        spec.auth = auth;
        let upstream = proxy::build_upstream(&spec, &credential, body)?;

        // The gate: no ledger entry, no egress.
        let start_id = self.ledger.append(&Event {
            kind: "remote.generate".into(),
            app_id: "host".into(),
            model: ledger_model.clone(),
            input_hash,
            preview: preview_of(&prompt_preview),
            status: "started".into(),
            detail,
            ..Default::default()
        })?;

        let started = Instant::now();
        let result = proxy::send(&self.http, &upstream).await;
        let duration_ms = started.elapsed().as_millis() as i64;
        match result {
            Ok(raw) => {
                let normalized = proxy::translate_response(spec.dialect, &raw);
                self.ledger.append(&Event {
                    kind: "remote.complete".into(),
                    app_id: "host".into(),
                    model: ledger_model,
                    status: "ok".into(),
                    detail: json!({"egress": "remote", "provider": spec.id}).to_string(),
                    ref_id: Some(start_id),
                    output_tokens: proxy::output_tokens(&normalized),
                    duration_ms,
                    ..Default::default()
                })?;
                Ok(normalized)
            }
            Err(e) => {
                self.ledger.append(&Event {
                    kind: "remote.complete".into(),
                    app_id: "host".into(),
                    model: ledger_model,
                    status: "error".into(),
                    detail: json!({
                        "egress": "remote",
                        "provider": spec.id,
                        "error": e.to_string(),
                    })
                    .to_string(),
                    ref_id: Some(start_id),
                    duration_ms,
                    ..Default::default()
                })?;
                Err(e.into())
            }
        }
    }
}
