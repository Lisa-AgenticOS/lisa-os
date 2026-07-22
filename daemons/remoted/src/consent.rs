//! Per-scope offload consent (PLAN §5.11: "a per-scope 'may offload'
//! switch in Settings"; default: nothing leaves the device). The broker
//! is the enforcement point — a remote request declares the context
//! scopes it carries, and any scope the user has not switched on refuses
//! the whole request (and the refusal is ledgered by the caller).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The offloadable context scopes, matching the §5.3 source taxonomy.
/// `prompt` is the user's own request text — even that does not leave
/// the device until explicitly enabled.
pub const SCOPES: &[&str] = &["prompt", "files", "mail", "calendar", "screen", "memory"];

#[derive(Debug, thiserror::Error)]
pub enum ConsentError {
    #[error("unknown scope: {0}")]
    UnknownScope(String),
    #[error("scope not permitted to leave this device: {0:?}")]
    Denied(Vec<String>),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("consent.toml: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConsentFile {
    #[serde(default)]
    may_offload: BTreeMap<String, bool>,
}

pub struct Consent {
    path: PathBuf,
    may_offload: BTreeMap<String, bool>,
}

impl Consent {
    pub fn open(state_dir: &Path) -> Result<Self, ConsentError> {
        std::fs::create_dir_all(state_dir)?;
        let path = state_dir.join("consent.toml");
        let may_offload = match std::fs::read_to_string(&path) {
            Ok(raw) => toml::from_str::<ConsentFile>(&raw)?.may_offload,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, may_offload })
    }

    /// Default is deny: an absent entry means "does not leave".
    pub fn allowed(&self, scope: &str) -> bool {
        self.may_offload.get(scope).copied().unwrap_or(false)
    }

    pub fn snapshot(&self) -> BTreeMap<String, bool> {
        SCOPES
            .iter()
            .map(|s| (s.to_string(), self.allowed(s)))
            .collect()
    }

    pub fn set(&mut self, scope: &str, allowed: bool) -> Result<(), ConsentError> {
        if !SCOPES.contains(&scope) {
            return Err(ConsentError::UnknownScope(scope.to_string()));
        }
        self.may_offload.insert(scope.to_string(), allowed);
        let raw = toml::to_string_pretty(&ConsentFile {
            may_offload: self.may_offload.clone(),
        })
        .expect("consent serializes");
        std::fs::write(&self.path, raw)?;
        Ok(())
    }

    /// Gate a request: every declared scope must be switched on, and the
    /// request must declare at least `prompt` (something always leaves —
    /// the prompt itself).
    pub fn check(&self, scopes: &[String]) -> Result<(), ConsentError> {
        for s in scopes {
            if !SCOPES.contains(&s.as_str()) {
                return Err(ConsentError::UnknownScope(s.clone()));
            }
        }
        let mut required: Vec<String> = scopes.to_vec();
        if !required.iter().any(|s| s == "prompt") {
            required.push("prompt".to_string());
        }
        let denied: Vec<String> = required.into_iter().filter(|s| !self.allowed(s)).collect();
        if denied.is_empty() {
            Ok(())
        } else {
            Err(ConsentError::Denied(denied))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_nothing_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let c = Consent::open(dir.path()).unwrap();
        for s in SCOPES {
            assert!(!c.allowed(s), "{s} must default to deny");
        }
        // Even a bare prompt-only request is refused by default.
        assert!(matches!(c.check(&[]), Err(ConsentError::Denied(d)) if d == ["prompt"]));
    }

    #[test]
    fn enabling_prompt_admits_prompt_only_requests() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Consent::open(dir.path()).unwrap();
        c.set("prompt", true).unwrap();
        assert!(c.check(&[]).is_ok());
        assert!(c.check(&["prompt".into()]).is_ok());
        // But not requests carrying an unconsented scope.
        let err = c.check(&["mail".into()]).unwrap_err();
        assert!(matches!(err, ConsentError::Denied(d) if d == ["mail"]));
    }

    #[test]
    fn consent_persists_and_unknown_scopes_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut c = Consent::open(dir.path()).unwrap();
            c.set("files", true).unwrap();
            c.set("prompt", true).unwrap();
            assert!(matches!(
                c.set("browser_history", true),
                Err(ConsentError::UnknownScope(_))
            ));
        }
        let c = Consent::open(dir.path()).unwrap();
        assert!(c.check(&["files".into()]).is_ok());
        assert!(matches!(
            c.check(&["telepathy".into()]),
            Err(ConsentError::UnknownScope(_))
        ));
    }
}
