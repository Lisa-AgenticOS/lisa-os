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
    #[error("{0:?} is not a credential name")]
    BadName(String),
}

/// Every path this store builds goes through here (issue #90).
///
/// `ClearKey` and `Logout` took a provider id straight off the wire and
/// interpolated it into a filename, so `../../victim/id_rsa` deleted a
/// file outside the store — over the socket, over D-Bus, and directly.
/// Other entry points happened to be safe because they looked the id up
/// in the registry first; that is a property of the callers, and callers
/// change.
///
/// So the check lives at the only place that turns a name into a path.
/// A credential name is one path component: no separators, no traversal,
/// nothing that resolves anywhere but this directory.
fn safe_name(name: &str) -> Result<&str, SecretsError> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        // A leading dot would also collide with the temp files
        // `set_named` writes next to the real ones.
        || name.starts_with('.');
    if bad {
        return Err(SecretsError::BadName(name.to_string()));
    }
    Ok(name)
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

    /// The one place a name becomes a path. Everything else in this
    /// store routes through it, so there is exactly one thing to get
    /// right (issue #90).
    fn path_of(&self, name: &str) -> Result<PathBuf, SecretsError> {
        Ok(self.dir.join(safe_name(name)?))
    }

    fn key_path(&self, provider: &str) -> Result<PathBuf, SecretsError> {
        // Validate the provider id itself, not the joined filename: the
        // suffix would otherwise make `..` look like the innocuous
        // `...key`, and the traversal would survive the check.
        safe_name(provider)?;
        self.path_of(&format!("{provider}.key"))
    }

    /// Store an API key (or serialized OAuth token set with the
    /// `<provider>.oauth.json` name via `set_named`). 0600 from birth:
    /// written to a same-directory temp file with restricted permissions,
    /// then atomically renamed.
    pub fn set(&self, provider: &str, value: &str) -> Result<(), SecretsError> {
        self.set_named(&format!("{provider}.key"), value)
    }

    pub fn set_named(&self, name: &str, value: &str) -> Result<(), SecretsError> {
        let dest = self.path_of(name)?;
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
        std::fs::rename(&tmp, dest)?;
        Ok(())
    }

    pub fn get_named(&self, name: &str) -> Result<String, SecretsError> {
        match std::fs::read_to_string(self.path_of(name)?) {
            Ok(v) => Ok(v.trim().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(SecretsError::Missing(name.to_string()))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn has_named(&self, name: &str) -> bool {
        self.path_of(name).is_ok_and(|p| p.is_file())
    }

    /// Remove a named credential (e.g. `<provider>.oauth.json` on
    /// logout). A missing file is a clean no-op — logout is idempotent.
    pub fn remove_named(&self, name: &str) -> Result<(), SecretsError> {
        match std::fs::remove_file(self.path_of(name)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get(&self, provider: &str) -> Result<String, SecretsError> {
        match std::fs::read_to_string(self.key_path(provider)?) {
            Ok(v) => Ok(v.trim().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(SecretsError::Missing(provider.to_string()))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn has(&self, provider: &str) -> bool {
        self.key_path(provider).is_ok_and(|p| p.is_file())
    }

    pub fn remove(&self, provider: &str) -> Result<(), SecretsError> {
        match std::fs::remove_file(self.key_path(provider)?) {
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

    /// Issue #90. `ClearKey("../../victim/id_rsa")` deleted a file
    /// outside the store — reachable over the socket, over D-Bus, and
    /// directly. The store must refuse before it touches the filesystem.
    #[test]
    fn a_traversing_name_cannot_reach_outside_the_store() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let s = SecretStore::open(&state).unwrap();

        // A file the attacker wants gone, one level up from the keys dir.
        let victim = root.path().join("id_rsa.key");
        std::fs::write(&victim, b"PRIVATE KEY").unwrap();

        for evil in [
            "../../id_rsa",
            "../id_rsa",
            "..",
            ".",
            "",
            "sub/id_rsa",
            r"..\id_rsa",
        ] {
            assert!(
                matches!(s.remove(evil), Err(SecretsError::BadName(_))),
                "remove({evil:?}) was not refused"
            );
            assert!(
                matches!(s.get(evil), Err(SecretsError::BadName(_))),
                "get({evil:?}) was not refused"
            );
            assert!(
                matches!(s.set(evil, "x"), Err(SecretsError::BadName(_))),
                "set({evil:?}) was not refused"
            );
            assert!(
                !s.has(evil),
                "has({evil:?}) claimed a file outside the store"
            );
        }
        assert!(victim.exists(), "a file outside the store was deleted");
    }

    /// The oauth path is the same hole with a different suffix
    /// (`Logout` → `<id>.oauth.json`), and it did not even have the
    /// registry lookup the key path accidentally relied on.
    #[test]
    fn the_named_variants_are_guarded_too() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let s = SecretStore::open(&state).unwrap();
        let victim = root.path().join("creds.oauth.json");
        std::fs::write(&victim, b"{}").unwrap();

        assert!(matches!(
            s.remove_named("../../creds.oauth.json"),
            Err(SecretsError::BadName(_))
        ));
        assert!(matches!(
            s.set_named("../../creds.oauth.json", "{}"),
            Err(SecretsError::BadName(_))
        ));
        assert!(matches!(
            s.get_named("../../creds.oauth.json"),
            Err(SecretsError::BadName(_))
        ));
        assert!(!s.has_named("../../creds.oauth.json"));
        assert_eq!(std::fs::read(&victim).unwrap(), b"{}");
    }

    /// The suffix is why the provider id is checked before it is joined:
    /// `..` + `.key` spells `...key`, which looks like an ordinary name.
    #[test]
    fn a_suffix_cannot_launder_a_traversal() {
        let root = tempfile::tempdir().unwrap();
        let s = SecretStore::open(&root.path().join("state")).unwrap();
        assert!(matches!(s.remove(".."), Err(SecretsError::BadName(_))));
        assert!(matches!(s.get("."), Err(SecretsError::BadName(_))));
    }

    /// And the ordinary names still work — a refusal that refuses
    /// everything is not a fix.
    #[test]
    fn real_provider_ids_are_unaffected() {
        let root = tempfile::tempdir().unwrap();
        let s = SecretStore::open(&root.path().join("state")).unwrap();
        for id in ["openai", "anthropic", "my-corp-llm", "llm.corp.example"] {
            s.set(id, "sk-1").unwrap();
            assert!(s.has(id), "{id} did not round-trip");
            assert_eq!(s.get(id).unwrap(), "sk-1");
            s.remove(id).unwrap();
        }
        s.set_named("anthropic.oauth.json", "{}").unwrap();
        assert!(s.has_named("anthropic.oauth.json"));
    }
}
