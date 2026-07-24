//! The broker core shared by the unix-socket HTTP surface (`api.rs`)
//! and the D-Bus management surface (`dbus.rs`). All policy lives here:
//! registry, credentials, consent, OAuth state, and — load-bearing —
//! the Ledger gate: every remote request is written to the Ledger
//! *before* egress (dataflow rule 4), with the `remote.` kind prefix as
//! the machine-readable "leaves your hardware" marking (§5.11).

use crate::consent::{Consent, ConsentError};
use crate::oauth::{self, LoginOutcome, OauthError, OauthManager};
use crate::proxy::{self, ProxyError};
use crate::registry::{AuthStyle, Dialect, Registry, RegistryError};
use crate::secrets::{SecretStore, SecretsError};
use lisa_ledger::{Event, Ledger, preview_of};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;

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
}

pub struct Broker {
    registry: Mutex<Registry>,
    secrets: SecretStore,
    consent: Mutex<Consent>,
    oauth: Arc<OauthManager>,
    ledger: Arc<Ledger>,
    http: reqwest::Client,
}

impl Broker {
    pub fn open(state_dir: &Path, ledger: Arc<Ledger>) -> anyhow::Result<Arc<Self>> {
        let secrets = SecretStore::open(state_dir)?;
        let http = reqwest::Client::new();
        Ok(Arc::new(Self {
            registry: Mutex::new(Registry::open(state_dir)?),
            oauth: OauthManager::new(http.clone(), secrets.clone()),
            consent: Mutex::new(Consent::open(state_dir)?),
            secrets,
            ledger,
            http,
        }))
    }

    /// Subscribe to OAuth login completions — the D-Bus layer forwards
    /// these as the `LoginCompleted` signal.
    pub fn subscribe_logins(&self) -> broadcast::Receiver<LoginOutcome> {
        self.oauth.subscribe()
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
                let oauth_capable = oauth::is_capable(&p.id);
                let connected = oauth_capable && self.oauth.connected(&p.id);
                let has_key = self.secrets.has(&p.id);
                json!({
                    "id": p.id,
                    "display_name": p.display_name,
                    "base_url": p.base_url,
                    // The wire auth style (bearer / anthropic-api-key / …).
                    "auth_style": p.auth,
                    "dialect": p.dialect,
                    "notes": p.notes,
                    "builtin": p.builtin,
                    "has_credential": has_key || connected,
                    // The active auth mode: OAuth takes precedence over a key.
                    "auth": if connected { "oauth" } else { "key" },
                    "oauth_capable": oauth_capable,
                    "connected": connected,
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

    // ---- Sign in with Claude / ChatGPT (OAuth) ----------------------------

    /// Start "Sign in with …": bind the loopback callback server, return
    /// the authorize URL for the panel to open in the browser. The broker
    /// never launches a browser (egress isolation). Only `anthropic` and
    /// `openai` are OAuth-capable (ADR-0010 §4).
    pub async fn begin_login(&self, provider_id: &str) -> Result<String, BrokerError> {
        // A registered provider only — a login for an unknown row is a
        // client error, not a callback we should bind a port for.
        self.registry
            .lock()
            .expect("registry lock")
            .get(provider_id)?;
        Ok(self.oauth.begin_login(provider_id).await?)
    }

    /// Forget a stored OAuth session (idempotent). Ledgered as a
    /// revocation of the egress capability the sign-in granted.
    pub fn logout(&self, provider_id: &str) -> Result<(), BrokerError> {
        self.oauth.logout(provider_id)?;
        self.ledger.append(&Event {
            kind: "remote.grant".into(),
            app_id: "settings".into(),
            model: format!("{provider_id}:oauth"),
            status: "revoked".into(),
            detail: json!({
                "egress": "remote",
                "provider": provider_id,
                "grant": "oauth-signin",
                "action": "revoke",
            })
            .to_string(),
            ..Default::default()
        })?;
        Ok(())
    }

    /// Ledger a completed sign-in as an egress-capability grant. Called
    /// by the D-Bus layer when a `LoginCompleted(ok=true)` arrives — the
    /// exchange itself already happened in the broker's own egress lane.
    pub fn ledger_login_grant(&self, provider_id: &str) {
        let _ = self.ledger.append(&Event {
            kind: "remote.grant".into(),
            app_id: "settings".into(),
            model: format!("{provider_id}:oauth"),
            status: "ok".into(),
            detail: json!({
                "egress": "remote",
                "provider": provider_id,
                "grant": "oauth-signin",
                "action": "grant",
            })
            .to_string(),
            ..Default::default()
        });
    }

    // ---- data plane --------------------------------------------------------

    /// Select the credential + wire auth style for `id`. OAuth takes
    /// precedence over any stored API key when a usable session exists
    /// (ADR-0010 §4): Anthropic → OAuth bearer (+beta header + Claude-Code
    /// framing); OpenAI OAuth rides the same plain `Authorization: Bearer`
    /// as its API key. Otherwise the stored key is used.
    async fn credential_for(
        &self,
        id: &str,
        auth: AuthStyle,
    ) -> Result<(String, AuthStyle), BrokerError> {
        if oauth::is_capable(id) && self.oauth.connected(id) {
            let access = self.oauth.access_token(id).await?;
            let style = match auth {
                AuthStyle::AnthropicApiKey | AuthStyle::AnthropicOauth => AuthStyle::AnthropicOauth,
                AuthStyle::Bearer => AuthStyle::Bearer,
            };
            return Ok((access, style));
        }
        match auth {
            AuthStyle::Bearer => Ok((self.secrets.get(id)?, AuthStyle::Bearer)),
            AuthStyle::AnthropicApiKey | AuthStyle::AnthropicOauth => {
                Ok((self.secrets.get(id)?, AuthStyle::AnthropicApiKey))
            }
        }
    }

    /// The provider's own live model list — its `/models` endpoint, never
    /// an invented catalogue (rule 8) — so Settings can offer a real model
    /// dropdown instead of a free-text box (construct's pattern, ADR-0010
    /// follow-up). Requires a stored credential. This is a metadata read,
    /// not a generation, so it is not ledgered as `remote.generate`.
    pub async fn list_models(&self, provider_id: &str) -> Result<Vec<String>, BrokerError> {
        let spec = self
            .registry
            .lock()
            .expect("registry lock")
            .get(provider_id)?;
        let base = spec.base_url.clone().ok_or_else(|| ProxyError::Upstream {
            status: 0,
            body: "provider has no configured endpoint".into(),
        })?;
        let (credential, auth) = self.credential_for(provider_id, spec.auth).await?;
        let base = base.trim_end_matches('/');
        // OpenAI-compat base_urls already carry /v1; Anthropic's does not.
        let url = match spec.dialect {
            Dialect::AnthropicMessages => format!("{base}/v1/models"),
            Dialect::OpenaiCompat => format!("{base}/models"),
        };
        let req = self.http.get(&url);
        let req = match auth {
            AuthStyle::AnthropicApiKey => req
                .header("x-api-key", &credential)
                .header("anthropic-version", "2023-06-01"),
            AuthStyle::AnthropicOauth => req
                .header("authorization", format!("Bearer {credential}"))
                .header("anthropic-version", "2023-06-01"),
            AuthStyle::Bearer => req.header("authorization", format!("Bearer {credential}")),
        };
        let resp = req.send().await.map_err(ProxyError::Http)?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ProxyError::Upstream {
                status: status.as_u16(),
                body,
            }
            .into());
        }
        let body: Value = resp.json().await.map_err(ProxyError::Http)?;
        let mut ids: Vec<String> = body["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        Ok(ids)
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

        let (credential, auth) = self.credential_for(&spec.id, spec.auth).await?;
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
