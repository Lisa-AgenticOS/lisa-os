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
    /// Register dev.lisaos.Inference1 on the session bus. Off by default in
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
    /// The model store's refs dir (e.g. /var/lib/lisa-models/refs). When
    /// set — and no explicit `model_path` — inferenced serves ANY model
    /// in it *by name*, lazily. This is the "download it in Settings and
    /// it's just there" path: no restart, no config per model.
    pub models_dir: Option<PathBuf>,
    /// Which model a bare/default request resolves to. Falls back to the
    /// first model present in `models_dir`, so a fresh download is usable
    /// without setting anything.
    pub default_model: Option<String>,
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
            models_dir: None,
            default_model: None,
            port: 7778,
            extra_args: Vec::new(),
            max_resident: 2,
        }
    }
}

/// First model file in a store refs dir (deterministic order), if any.
/// Lets inferenced serve a just-downloaded model with no default set.
pub fn first_model_in(dir: &Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| !t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names.into_iter().next()
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
    fn models_dir_and_default_parse_and_first_model_is_deterministic() {
        let c: Config = toml::from_str(
            "engine = \"llama\"\n[llama]\nmodels_dir = \"/var/lib/lisa-models/refs\"\n",
        )
        .unwrap();
        assert_eq!(
            c.llama.models_dir.as_deref(),
            Some(Path::new("/var/lib/lisa-models/refs"))
        );
        assert!(c.llama.default_model.is_none());

        // first_model_in: empty dir → None; picks the alphabetically-first
        // regular file (so a fresh download is servable with no default).
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(first_model_in(dir.path()), None);
        std::fs::write(dir.path().join("qwen3-0.6b-instruct-q8"), b"x").unwrap();
        std::fs::write(dir.path().join("gemma-3-1b-it-q8"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("blobs")).unwrap(); // dirs skipped
        assert_eq!(
            first_model_in(dir.path()).as_deref(),
            Some("gemma-3-1b-it-q8")
        );
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
