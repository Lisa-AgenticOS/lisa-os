//! Daemon configuration. Defaults are dev-friendly; production hardening
//! (systemd sandbox, PrivateNetwork) lives in the unit files under
//! `os/packages/`, not here (`docs/PLAN.md` §5.1).

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub bind: Bind,
    pub engine: EngineKind,
    /// Register org.lisa.Inference1 on the session bus. Off by default in
    /// M0 — the full D-Bus surface is an M1 deliverable.
    pub dbus: bool,
    pub llama: LlamaConfig,
    pub remote: RemoteConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct Bind(pub String);

impl Default for Bind {
    fn default() -> Self {
        // The OpenAI-compat port from PLAN §5.1.
        Self("127.0.0.1:7777".to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    /// Deterministic echo engine for tests and plumbing demos.
    #[default]
    Stub,
    /// Supervised llama-server child (wired fully in M1).
    Llama,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LlamaConfig {
    /// llama-server binary; resolved via PATH by default.
    pub server_bin: PathBuf,
    pub model_path: Option<PathBuf>,
    /// Port the supervised child listens on (loopback only).
    pub port: u16,
    /// Extra llama-server arguments (e.g. ["-ngl", "99"] for GPU offload).
    pub extra_args: Vec<String>,
    /// Resident-model cap: children beyond this are LRU-evicted (§5.1).
    pub max_resident: usize,
}

impl Default for LlamaConfig {
    fn default() -> Self {
        Self {
            server_bin: PathBuf::from("llama-server"),
            model_path: None,
            port: 7778,
            extra_args: Vec::new(),
            max_resident: 2,
        }
    }
}

/// The `remote:byo` tier surface (PLAN §5.11, ADR-0008). inferenced
/// itself never gets network (CLAUDE.md rule 5): remote requests are
/// handed to the lisa-remoted broker over a local unix socket. This is
/// config/routing surface only — the wiring that forwards requests
/// lands with the broker integration; nothing here opens a network
/// connection.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteConfig {
    /// Off by default: with the tier disabled, `remote:byo:*` model
    /// hints are refused locally and nothing can leave the device.
    pub enabled: bool,
    /// lisa-remoted's unix socket (AF_UNIX — already permitted by the
    /// hardened unit's RestrictAddressFamilies).
    pub socket: Option<PathBuf>,
}

/// Model hints of this shape route to the broker:
/// `remote:byo:<provider>:<model>` (e.g. `remote:byo:tinker:tinker://...`).
pub const REMOTE_BYO_PREFIX: &str = "remote:byo:";

/// Split a `remote:byo:` model hint into (provider, model), if it is one.
pub fn parse_remote_hint(hint: &str) -> Option<(&str, &str)> {
    let rest = hint.strip_prefix(REMOTE_BYO_PREFIX)?;
    let (provider, model) = rest.split_once(':')?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some((provider, model))
}

impl Config {
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        match path {
            Some(p) => {
                let raw = std::fs::read_to_string(p)
                    .map_err(|e| anyhow::anyhow!("reading config {}: {e}", p.display()))?;
                Ok(toml::from_str(&raw)?)
            }
            None => Ok(Self::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_bind_loopback_7777_with_stub_engine() {
        let c = Config::default();
        assert_eq!(c.bind.0, "127.0.0.1:7777");
        assert_eq!(c.engine, EngineKind::Stub);
        assert!(!c.dbus);
    }

    #[test]
    fn parses_partial_toml() {
        let c: Config = toml::from_str("engine = \"llama\"\n[llama]\nport = 9000\n").unwrap();
        assert_eq!(c.engine, EngineKind::Llama);
        assert_eq!(c.llama.port, 9000);
        assert_eq!(c.bind.0, "127.0.0.1:7777");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<Config>("no_such_key = 1\n").is_err());
    }

    #[test]
    fn remote_tier_is_disabled_by_default_and_parses_from_toml() {
        let c = Config::default();
        assert!(!c.remote.enabled, "nothing leaves the device by default");
        assert!(c.remote.socket.is_none());

        let c: Config =
            toml::from_str("[remote]\nenabled = true\nsocket = \"/run/lisa/remoted.sock\"\n")
                .unwrap();
        assert!(c.remote.enabled);
        assert_eq!(
            c.remote.socket.as_deref(),
            Some(Path::new("/run/lisa/remoted.sock"))
        );
    }

    #[test]
    fn remote_byo_model_hints_parse_provider_and_model() {
        assert_eq!(
            parse_remote_hint("remote:byo:openai:gpt-x"),
            Some(("openai", "gpt-x"))
        );
        // Tinker models are URIs containing colons; only the first
        // separator after the provider splits.
        assert_eq!(
            parse_remote_hint("remote:byo:tinker:tinker://run:train:0/w/5"),
            Some(("tinker", "tinker://run:train:0/w/5"))
        );
        assert_eq!(parse_remote_hint("qwen3-0.6b"), None);
        assert_eq!(parse_remote_hint("remote:byo:"), None);
        assert_eq!(parse_remote_hint("remote:byo:openai"), None);
    }
}
