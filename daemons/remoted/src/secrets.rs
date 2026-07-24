//! Credential store (ADR-0008 §3): one mode-0600 file per credential in
//! a mode-0700 directory owned by the daemon user. Boring, auditable,
//! and never world-readable. Values are write-only through every API
//! surface — callers can learn *presence*, never the value (only the
//! proxy path reads it back to authenticate an upstream request).

use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    #[error("no credential stored for {0}")]
    Missing(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct SecretStore {
    dir: PathBuf,
}

impl SecretStore {
    pub fn open(state_dir: &Path) -> Result<Self, SecretsError> {
        let dir = state_dir.join("keys");
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { dir })
    }

    fn key_path(&self, provider: &str) -> PathBuf {
        self.dir.join(format!("{provider}.key"))
    }

    /// Store an API key (or serialized OAuth token set with the
    /// `<provider>.oauth.json` name via `set_named`). 0600 from birth:
    /// written to a same-directory temp file with restricted permissions,
    /// then atomically renamed.
    pub fn set(&self, provider: &str, value: &str) -> Result<(), SecretsError> {
        self.set_named(&format!("{provider}.key"), value)
    }

    pub fn set_named(&self, name: &str, value: &str) -> Result<(), SecretsError> {
        let tmp = self.dir.join(format!(".{name}.tmp"));
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            f.write_all(value.trim().as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, self.dir.join(name))?;
        Ok(())
    }

    pub fn get_named(&self, name: &str) -> Result<String, SecretsError> {
        match std::fs::read_to_string(self.dir.join(name)) {
            Ok(v) => Ok(v.trim().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(SecretsError::Missing(name.to_string()))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn has_named(&self, name: &str) -> bool {
        self.dir.join(name).is_file()
    }

    /// Remove a named credential (e.g. `<provider>.oauth.json` on
    /// logout). A missing file is a clean no-op — logout is idempotent.
    pub fn remove_named(&self, name: &str) -> Result<(), SecretsError> {
        match std::fs::remove_file(self.dir.join(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get(&self, provider: &str) -> Result<String, SecretsError> {
        match std::fs::read_to_string(self.key_path(provider)) {
            Ok(v) => Ok(v.trim().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(SecretsError::Missing(provider.to_string()))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn has(&self, provider: &str) -> bool {
        self.key_path(provider).is_file()
    }

    pub fn remove(&self, provider: &str) -> Result<(), SecretsError> {
        match std::fs::remove_file(self.key_path(provider)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(SecretsError::Missing(provider.to_string()))
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_remove_round_trip_with_trimming() {
        let dir = tempfile::tempdir().unwrap();
        let s = SecretStore::open(dir.path()).unwrap();
        assert!(!s.has("openai"));
        s.set("openai", "sk-test-123\n").unwrap();
        assert!(s.has("openai"));
        assert_eq!(s.get("openai").unwrap(), "sk-test-123");
        s.remove("openai").unwrap();
        assert!(matches!(s.get("openai"), Err(SecretsError::Missing(_))));
    }

    #[cfg(unix)]
    #[test]
    fn key_files_are_0600_and_dir_is_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let s = SecretStore::open(dir.path()).unwrap();
        s.set("tinker", "tk-secret").unwrap();
        let dmode = std::fs::metadata(dir.path().join("keys"))
            .unwrap()
            .permissions()
            .mode();
        let fmode = std::fs::metadata(dir.path().join("keys/tinker.key"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dmode & 0o777, 0o700, "keys dir must be 0700");
        assert_eq!(fmode & 0o777, 0o600, "key file must be 0600");
    }
}
