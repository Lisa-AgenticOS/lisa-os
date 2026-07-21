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
}

impl Default for LlamaConfig {
    fn default() -> Self {
        Self {
            server_bin: PathBuf::from("llama-server"),
            model_path: None,
            port: 7778,
            extra_args: Vec::new(),
        }
    }
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
}
