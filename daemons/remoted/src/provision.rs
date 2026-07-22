//! Field-test key provisioning from the ESP (ADR-0008 §6, provisional
//! until the M7 OOBE).
//!
//! The dev machine stages `lisa-provision/<provider>.key` on the FAT ESP
//! — the only partition writable from macOS. FAT is world-readable, so
//! the ESP is a *staging area, never the store*: this import moves each
//! key into the broker's 0600 secrets store, overwrites the staged file
//! with zeros (best-effort on FAT — the honest guarantee is "no longer
//! present", not forensic erasure), and removes it. Run as a oneshot
//! unit before the broker starts (`lisa-remoted --import-esp /efi`).

use crate::registry::Registry;
use crate::secrets::SecretStore;
use std::io::{Seek, Write};
use std::path::Path;

pub const PROVISION_DIR: &str = "lisa-provision";

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Imported(String),
    /// File stem matched no registry id; left in place for a human.
    UnknownProvider(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("secrets: {0}")]
    Secrets(#[from] crate::secrets::SecretsError),
}

/// Import staged keys from `esp_root/lisa-provision`. Missing directory
/// is a clean no-op (the unit is also gated by ConditionPathIsDirectory).
pub fn import_esp(
    esp_root: &Path,
    registry: &Registry,
    secrets: &SecretStore,
) -> Result<Vec<Outcome>, ProvisionError> {
    let dir = esp_root.join(PROVISION_DIR);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut outcomes = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("key") || !path.is_file() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        if registry.get(&stem).is_err() {
            tracing::warn!(provider = %stem, "staged key matches no provider id; leaving in place");
            outcomes.push(Outcome::UnknownProvider(stem));
            continue;
        }
        let value = std::fs::read_to_string(&path)?;
        let value = value.trim();
        if value.is_empty() {
            tracing::warn!(provider = %stem, "staged key file is empty; removing");
        } else {
            secrets.set(&stem, value)?;
        }
        scrub_and_remove(&path)?;
        outcomes.push(Outcome::Imported(stem));
    }
    // Leave no staging residue behind when everything was consumed.
    if std::fs::read_dir(&dir)?.next().is_none() {
        let _ = std::fs::remove_dir(&dir);
    }
    Ok(outcomes)
}

/// Overwrite with zeros, fsync, then unlink (best effort on FAT).
fn scrub_and_remove(path: &Path) -> std::io::Result<()> {
    let len = std::fs::metadata(path)?.len();
    {
        let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
        f.rewind()?;
        let zeros = vec![0u8; len as usize];
        f.write_all(&zeros)?;
        f.sync_all()?;
    }
    std::fs::remove_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Registry, SecretStore, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let registry = Registry::open(&state).unwrap();
        let secrets = SecretStore::open(&state).unwrap();
        let esp = dir.path().join("esp");
        std::fs::create_dir_all(esp.join(PROVISION_DIR)).unwrap();
        (dir, registry, secrets, esp)
    }

    #[test]
    fn imports_known_keys_into_the_store_and_scrubs_the_esp() {
        let (_dir, registry, secrets, esp) = fixture();
        let staged = esp.join(PROVISION_DIR).join("tinker.key");
        std::fs::write(&staged, "tk-fieldtest-1\n").unwrap();

        let outcomes = import_esp(&esp, &registry, &secrets).unwrap();
        assert_eq!(outcomes, vec![Outcome::Imported("tinker".into())]);
        assert_eq!(secrets.get("tinker").unwrap(), "tk-fieldtest-1");
        assert!(
            !staged.exists(),
            "the ESP is a staging area, never the store"
        );
        assert!(
            !esp.join(PROVISION_DIR).exists(),
            "emptied staging dir is removed"
        );
    }

    #[test]
    fn unknown_provider_ids_are_left_in_place() {
        let (_dir, registry, secrets, esp) = fixture();
        let staged = esp.join(PROVISION_DIR).join("mystery.key");
        std::fs::write(&staged, "??").unwrap();

        let outcomes = import_esp(&esp, &registry, &secrets).unwrap();
        assert_eq!(outcomes, vec![Outcome::UnknownProvider("mystery".into())]);
        assert!(staged.exists(), "unknown files are for a human to inspect");
        assert!(!secrets.has("mystery"));
    }

    #[test]
    fn missing_staging_dir_is_a_clean_noop() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let registry = Registry::open(&state).unwrap();
        let secrets = SecretStore::open(&state).unwrap();
        let outcomes = import_esp(&dir.path().join("no-esp"), &registry, &secrets).unwrap();
        assert!(outcomes.is_empty());
    }

    #[test]
    fn multiple_keys_import_in_one_pass() {
        let (_dir, registry, secrets, esp) = fixture();
        for (name, val) in [("together.key", "tg-1"), ("fireworks.key", "fw-1")] {
            std::fs::write(esp.join(PROVISION_DIR).join(name), val).unwrap();
        }
        let mut outcomes = import_esp(&esp, &registry, &secrets).unwrap();
        outcomes.sort_by_key(|o| format!("{o:?}"));
        assert_eq!(outcomes.len(), 2);
        assert_eq!(secrets.get("together").unwrap(), "tg-1");
        assert_eq!(secrets.get("fireworks").unwrap(), "fw-1");
    }
}
