//! Data-driven provider registry (ADR-0008 §2).
//!
//! Providers are rows, not code: built-in rows for the endpoints we have
//! verified against provider documentation (CLAUDE.md rule 8 — sources
//! cited inline), plus user-supplied custom OpenAI-compatible rows
//! persisted in `providers.toml` under the broker state dir (§5.11:
//! "an OpenAI-compat URL the user supplies").

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How the credential rides on the upstream request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthStyle {
    /// `Authorization: Bearer <key>` (OpenAI, Tinker, Together, Fireworks).
    Bearer,
    /// `x-api-key: <key>` + `anthropic-version: 2023-06-01`.
    /// Source: platform.claude.com/docs/en/manage-claude/authentication.
    AnthropicApiKey,
    /// `Authorization: Bearer <oauth token>` + `anthropic-beta:
    /// oauth-2025-04-20` (Sign in with Claude; see `oauth.rs`).
    AnthropicOauth,
}

/// The wire dialect the provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Dialect {
    /// `POST {base_url}/chat/completions`, OpenAI request/response shape.
    OpenaiCompat,
    /// Native `POST {base_url}/v1/messages` (Anthropic). The broker
    /// translates: Anthropic's own OpenAI-compat layer is documented as
    /// test-only and drops guaranteed schema conformance (`strict` /
    /// `response_format` ignored), which would break guided generation —
    /// so we use the native API where compat lies (ADR-0008 §2).
    AnthropicMessages,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSpec {
    pub id: String,
    pub display_name: String,
    /// None only when an entry is registered but its endpoint is not yet
    /// configured — never guessed (rule 8).
    pub base_url: Option<String>,
    pub auth: AuthStyle,
    pub dialect: Dialect,
    /// Human-readable caveats surfaced in Settings.
    pub notes: String,
    #[serde(default)]
    pub builtin: bool,
}

/// Built-in rows. Every URL below was verified against the provider's
/// public documentation on 2026-07-22 (citations in ADR-0008).
pub fn builtin_providers() -> Vec<ProviderSpec> {
    vec![
        ProviderSpec {
            id: "openai".into(),
            display_name: "OpenAI".into(),
            // developers.openai.com/api/reference/overview
            base_url: Some("https://api.openai.com/v1".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "OpenAI API (chat completions).".into(),
            builtin: true,
        },
        ProviderSpec {
            id: "anthropic".into(),
            display_name: "Anthropic".into(),
            // platform.claude.com/docs/en/manage-claude/authentication
            base_url: Some("https://api.anthropic.com".into()),
            auth: AuthStyle::AnthropicApiKey,
            dialect: Dialect::AnthropicMessages,
            notes: "Native Messages API; Sign in with Claude OAuth once \
                    Anthropic publishes a registerable client (ADR-0008 §4)."
                .into(),
            builtin: true,
        },
        ProviderSpec {
            id: "tinker".into(),
            display_name: "Tinker (Thinking Machines)".into(),
            // tinker-docs.thinkingmachines.ai/tinker/compatible-apis/openai/
            base_url: Some(
                "https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1".into(),
            ),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "OpenAI-compatible sampling (beta); models are tinker:// \
                    checkpoint URIs. The same credential serves the M6 \
                    adapter-training lane."
                .into(),
            builtin: true,
        },
        ProviderSpec {
            id: "together".into(),
            display_name: "Together.ai".into(),
            // docs.together.ai/docs/openai-api-compatibility
            base_url: Some("https://api.together.ai/v1".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "OpenAI-compatible; namespaced model ids (org/model).".into(),
            builtin: true,
        },
        ProviderSpec {
            id: "fireworks".into(),
            display_name: "Fireworks.ai".into(),
            // docs.fireworks.ai/tools-sdks/openai-compatibility
            base_url: Some("https://api.fireworks.ai/inference/v1".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "OpenAI-compatible chat completions.".into(),
            builtin: true,
        },
    ]
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("unknown provider: {0}")]
    Unknown(String),
    #[error("provider id already exists: {0}")]
    Exists(String),
    #[error("built-in providers cannot be removed: {0}")]
    Builtin(String),
    #[error("invalid provider id {0:?}: lowercase letters, digits, '-', '_' only")]
    InvalidId(String),
    #[error("invalid base_url {0:?}: must start with https:// (or http:// for tests)")]
    InvalidUrl(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("providers.toml: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CustomFile {
    #[serde(default)]
    providers: Vec<ProviderSpec>,
}

/// Registry = built-in table + persisted custom rows.
pub struct Registry {
    path: PathBuf,
    custom: Vec<ProviderSpec>,
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

impl Registry {
    pub fn open(state_dir: &std::path::Path) -> Result<Self, RegistryError> {
        std::fs::create_dir_all(state_dir)?;
        let path = state_dir.join("providers.toml");
        let custom = match std::fs::read_to_string(&path) {
            Ok(raw) => toml::from_str::<CustomFile>(&raw)?.providers,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, custom })
    }

    pub fn list(&self) -> Vec<ProviderSpec> {
        let mut all = builtin_providers();
        all.extend(self.custom.iter().cloned());
        all
    }

    pub fn get(&self, id: &str) -> Result<ProviderSpec, RegistryError> {
        self.list()
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| RegistryError::Unknown(id.to_string()))
    }

    /// Register a user-supplied OpenAI-compatible endpoint (§5.11).
    pub fn add_custom(
        &mut self,
        id: &str,
        display_name: &str,
        base_url: &str,
    ) -> Result<(), RegistryError> {
        if !valid_id(id) {
            return Err(RegistryError::InvalidId(id.to_string()));
        }
        if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
            return Err(RegistryError::InvalidUrl(base_url.to_string()));
        }
        if self.list().iter().any(|p| p.id == id) {
            return Err(RegistryError::Exists(id.to_string()));
        }
        self.custom.push(ProviderSpec {
            id: id.to_string(),
            display_name: display_name.to_string(),
            base_url: Some(base_url.trim_end_matches('/').to_string()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "User-supplied OpenAI-compatible endpoint.".into(),
            builtin: false,
        });
        self.persist()
    }

    pub fn remove_custom(&mut self, id: &str) -> Result<(), RegistryError> {
        if builtin_providers().iter().any(|p| p.id == id) {
            return Err(RegistryError::Builtin(id.to_string()));
        }
        let before = self.custom.len();
        self.custom.retain(|p| p.id != id);
        if self.custom.len() == before {
            return Err(RegistryError::Unknown(id.to_string()));
        }
        self.persist()
    }

    fn persist(&self) -> Result<(), RegistryError> {
        let file = CustomFile {
            providers: self.custom.clone(),
        };
        let raw = toml::to_string_pretty(&file).expect("provider rows serialize");
        std::fs::write(&self.path, raw)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_table_has_the_five_verified_providers() {
        let ids: Vec<String> = builtin_providers().into_iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            ["openai", "anthropic", "tinker", "together", "fireworks"]
        );
        for p in builtin_providers() {
            assert!(p.base_url.is_some(), "{} must have a verified URL", p.id);
            assert!(p.builtin);
        }
    }

    #[test]
    fn anthropic_is_native_dialect_not_compat() {
        let a = builtin_providers()
            .into_iter()
            .find(|p| p.id == "anthropic")
            .unwrap();
        assert_eq!(a.dialect, Dialect::AnthropicMessages);
        assert_eq!(a.auth, AuthStyle::AnthropicApiKey);
    }

    #[test]
    fn custom_provider_round_trips_through_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Registry::open(dir.path()).unwrap();
        r.add_custom("homelab", "Homelab llama", "http://10.0.0.2:8080/v1/")
            .unwrap();
        // Trailing slash normalized, row visible.
        assert_eq!(
            r.get("homelab").unwrap().base_url.as_deref(),
            Some("http://10.0.0.2:8080/v1")
        );

        let r2 = Registry::open(dir.path()).unwrap();
        assert!(r2.get("homelab").is_ok(), "custom row must persist");
        assert_eq!(r2.list().len(), builtin_providers().len() + 1);
    }

    #[test]
    fn rejects_bad_ids_duplicates_and_builtin_removal() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Registry::open(dir.path()).unwrap();
        assert!(matches!(
            r.add_custom("Bad Id", "x", "https://x"),
            Err(RegistryError::InvalidId(_))
        ));
        assert!(matches!(
            r.add_custom("openai", "x", "https://x"),
            Err(RegistryError::Exists(_))
        ));
        assert!(matches!(
            r.add_custom("x", "x", "ftp://x"),
            Err(RegistryError::InvalidUrl(_))
        ));
        assert!(matches!(
            r.remove_custom("tinker"),
            Err(RegistryError::Builtin(_))
        ));
        assert!(matches!(
            r.remove_custom("nope"),
            Err(RegistryError::Unknown(_))
        ));
    }
}
