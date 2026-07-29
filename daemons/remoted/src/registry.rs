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
    /// The user said this endpoint is their own machine or their own
    /// network, so loopback/RFC1918 and plaintext `http` are allowed
    /// *for this row* (issue #92).
    ///
    /// Per provider, never per daemon: a switch that turned this on
    /// globally so somebody could reach their Ollama box would re-open
    /// the hole for every other provider at the same moment. Defaults
    /// off, and `#[serde(default)]` means a row written before this
    /// field existed reads back as public-only.
    #[serde(default)]
    pub allow_local: bool,
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
            allow_local: false,
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
            allow_local: false,
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
            allow_local: false,
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
            allow_local: false,
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
            allow_local: false,
        },
        ProviderSpec {
            id: "huggingface".into(),
            display_name: "Hugging Face".into(),
            // huggingface.co/docs/inference-providers — the OpenAI-compat
            // router; one HF token fans out to many upstream providers.
            base_url: Some("https://router.huggingface.co/v1".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "Inference Providers router (chat only). Model ids are \
                    org/model with an optional policy/provider suffix, e.g. \
                    openai/gpt-oss-120b:cheapest or :groq. HF token from \
                    hf.co/settings/tokens with Inference Providers scope."
                .into(),
            builtin: true,
            allow_local: false,
        },
        ProviderSpec {
            id: "moonshot".into(),
            display_name: "Moonshot (Kimi)".into(),
            // platform.moonshot.ai/docs/api — OpenAI-compatible (.cn for
            // the China endpoint).
            base_url: Some("https://api.moonshot.ai/v1".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "Kimi models (kimi-k2, moonshot-v1-*), OpenAI-compatible.".into(),
            builtin: true,
            allow_local: false,
        },
        ProviderSpec {
            id: "google".into(),
            display_name: "Google Gemini".into(),
            // ai.google.dev/gemini-api/docs/openai — the OpenAI-compat shim
            // over the Gemini API; the key is a bog-standard Bearer token.
            base_url: Some("https://generativelanguage.googleapis.com/v1beta/openai".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "Gemini via its OpenAI-compatible endpoint (model ids like \
                    gemini-2.5-flash)."
                .into(),
            builtin: true,
            allow_local: false,
        },
        ProviderSpec {
            id: "deepseek".into(),
            display_name: "DeepSeek".into(),
            // api-docs.deepseek.com — OpenAI-compatible.
            base_url: Some("https://api.deepseek.com/v1".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "OpenAI-compatible (deepseek-chat, deepseek-reasoner).".into(),
            builtin: true,
            allow_local: false,
        },
        ProviderSpec {
            id: "groq".into(),
            display_name: "Groq".into(),
            // console.groq.com/docs/openai — OpenAI-compatible, very fast.
            base_url: Some("https://api.groq.com/openai/v1".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "OpenAI-compatible; low-latency inference.".into(),
            builtin: true,
            allow_local: false,
        },
        ProviderSpec {
            id: "mistral".into(),
            display_name: "Mistral".into(),
            // docs.mistral.ai — OpenAI-compatible.
            base_url: Some("https://api.mistral.ai/v1".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "OpenAI-compatible chat completions.".into(),
            builtin: true,
            allow_local: false,
        },
        ProviderSpec {
            id: "xai".into(),
            display_name: "xAI (Grok)".into(),
            // docs.x.ai — OpenAI-compatible.
            base_url: Some("https://api.x.ai/v1".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "Grok models, OpenAI-compatible.".into(),
            builtin: true,
            allow_local: false,
        },
        ProviderSpec {
            id: "openrouter".into(),
            display_name: "OpenRouter".into(),
            // openrouter.ai/docs — OpenAI-compatible aggregator across
            // many upstream providers.
            base_url: Some("https://openrouter.ai/api/v1".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "Aggregator; model ids are vendor/model (e.g. anthropic/claude-3.5-sonnet)."
                .into(),
            builtin: true,
            allow_local: false,
        },
        ProviderSpec {
            id: "perplexity".into(),
            display_name: "Perplexity".into(),
            // docs.perplexity.ai — OpenAI-compatible (sonar models).
            base_url: Some("https://api.perplexity.ai".into()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: "Sonar models, OpenAI-compatible.".into(),
            builtin: true,
            allow_local: false,
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
    #[error("invalid base_url: {0}")]
    InvalidUrl(#[from] crate::net::UrlRefusal),
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
    ///
    /// `locality` is the user's answer to "is this your own machine?".
    /// It decides whether loopback/LAN addresses and plaintext `http`
    /// are acceptable *for this row* — see `crate::net`.
    pub fn add_custom(
        &mut self,
        id: &str,
        display_name: &str,
        base_url: &str,
        locality: crate::net::Locality,
    ) -> Result<(), RegistryError> {
        if !valid_id(id) {
            return Err(RegistryError::InvalidId(id.to_string()));
        }
        // Store what was validated, not what was typed: the parser
        // normalises, and a second string is a second thing to check.
        let url = crate::net::validate_base_url(base_url, locality)?;
        if self.list().iter().any(|p| p.id == id) {
            return Err(RegistryError::Exists(id.to_string()));
        }
        self.custom.push(ProviderSpec {
            id: id.to_string(),
            display_name: display_name.to_string(),
            base_url: Some(url.as_str().trim_end_matches('/').to_string()),
            auth: AuthStyle::Bearer,
            dialect: Dialect::OpenaiCompat,
            notes: if locality == crate::net::Locality::LocalAllowed {
                "User-supplied endpoint on this machine or this network.".into()
            } else {
                "User-supplied OpenAI-compatible endpoint.".into()
            },
            builtin: false,
            allow_local: locality == crate::net::Locality::LocalAllowed,
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
        // 0600, not `fs::write`'s 0666 & ~umask (issue #109). Under the
        // user unit this file lives in the home directory, where 0644
        // means any local user who can traverse it reads whatever a row
        // happens to contain. Same temp-file-then-rename shape as
        // SecretStore, so the file is never briefly world-readable.
        let tmp = self.path.with_extension("toml.tmp");
        {
            use std::io::Write;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            f.write_all(raw.as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_table_has_the_verified_providers() {
        let ids: Vec<String> = builtin_providers().into_iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                "openai",
                "anthropic",
                "tinker",
                "together",
                "fireworks",
                "huggingface",
                "moonshot",
                "google",
                "deepseek",
                "groq",
                "mistral",
                "xai",
                "openrouter",
                "perplexity",
            ]
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
        // A box on the LAN — which is now something the user has to say
        // out loud (issue #92). This very test used to register it by
        // default, which is the habit the fix is about.
        r.add_custom(
            "homelab",
            "Homelab llama",
            "http://10.0.0.2:8080/v1/",
            crate::net::Locality::LocalAllowed,
        )
        .unwrap();
        // Trailing slash normalized, row visible.
        assert_eq!(
            r.get("homelab").unwrap().base_url.as_deref(),
            Some("http://10.0.0.2:8080/v1")
        );
        assert!(r.get("homelab").unwrap().allow_local);

        let r2 = Registry::open(dir.path()).unwrap();
        assert!(r2.get("homelab").is_ok(), "custom row must persist");
        assert!(
            r2.get("homelab").unwrap().allow_local,
            "the local-endpoint decision must survive a restart — otherwise \
             the row silently starts being dialled through the DNS guard"
        );
        assert_eq!(r2.list().len(), builtin_providers().len() + 1);
    }

    /// The same URL without that answer is refused (issue #92): the
    /// broker is the one process with egress, and pointing it inward is
    /// a decision, not a default.
    #[test]
    fn a_lan_endpoint_is_refused_unless_the_user_says_it_is_theirs() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Registry::open(dir.path()).unwrap();
        assert!(matches!(
            r.add_custom(
                "homelab",
                "Homelab llama",
                "http://10.0.0.2:8080/v1/",
                crate::net::Locality::PublicOnly
            ),
            Err(RegistryError::InvalidUrl(_))
        ));
        assert!(r.get("homelab").is_err(), "the row was written anyway");
    }

    /// Issue #109: providers.toml holds whatever the user typed, and
    /// under the user unit it sits in the home directory. `fs::write`
    /// made it 0644.
    #[cfg(unix)]
    #[test]
    fn the_provider_file_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mut r = Registry::open(dir.path()).unwrap();
        r.add_custom(
            "corp",
            "Corp",
            "https://llm.corp.example/v1",
            crate::net::Locality::PublicOnly,
        )
        .unwrap();
        let mode = std::fs::metadata(dir.path().join("providers.toml"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "providers.toml must be 0600");
    }

    #[test]
    fn rejects_bad_ids_duplicates_and_builtin_removal() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Registry::open(dir.path()).unwrap();
        assert!(matches!(
            r.add_custom(
                "Bad Id",
                "x",
                "https://x.example",
                crate::net::Locality::PublicOnly
            ),
            Err(RegistryError::InvalidId(_))
        ));
        assert!(matches!(
            r.add_custom(
                "openai",
                "x",
                "https://x.example",
                crate::net::Locality::PublicOnly
            ),
            Err(RegistryError::Exists(_))
        ));
        assert!(matches!(
            r.add_custom(
                "x",
                "x",
                "ftp://x.example",
                crate::net::Locality::PublicOnly
            ),
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
