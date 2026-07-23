//! Content-addressed model store: blake3-addressed blobs with hardlinked
//! human-readable names (`docs/PLAN.md` §5.2).
//!
//! Layout under the store root:
//!   blobs/<blake3-hex>   — the actual weights, one file per unique content
//!   refs/<name>          — hardlink into blobs/, the name apps/catalog use
//!
//! Dedupe falls out of content addressing: two refs to identical bytes share
//! one blob. `gc` removes blobs whose only remaining link is the blob entry
//! itself.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("ref `{0}` already exists")]
    RefExists(String),
    #[error("ref `{0}` not found")]
    RefNotFound(String),
    #[error("hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
}

fn io_err(path: &Path, source: io::Error) -> StoreError {
    StoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, Clone)]
pub struct RefEntry {
    pub name: String,
    pub blake3: String,
    pub size: u64,
}

#[derive(Debug, Default)]
pub struct VerifyReport {
    pub ok: usize,
    /// (blob path, expected hash from filename, actual recomputed hash)
    pub corrupt: Vec<(PathBuf, String, String)>,
}

impl VerifyReport {
    pub fn is_clean(&self) -> bool {
        self.corrupt.is_empty()
    }
}

pub struct ModelStore {
    root: PathBuf,
}

impl ModelStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        for dir in [root.join("blobs"), root.join("refs"), root.join("tmp")] {
            fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs")
    }

    fn refs_dir(&self) -> PathBuf {
        self.root.join("refs")
    }

    pub(crate) fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// Streaming blake3 of a file on disk.
    pub fn hash_file(path: &Path) -> Result<String, StoreError> {
        let file = fs::File::open(path).map_err(|e| io_err(path, e))?;
        let mut hasher = blake3::Hasher::new();
        hasher
            .update_reader(io::BufReader::new(file))
            .map_err(|e| io_err(path, e))?;
        Ok(hasher.finalize().to_hex().to_string())
    }

    /// Ingest `src` under `name`. The source file is copied, never removed.
    pub fn add_file(&self, src: &Path, name: &str) -> Result<RefEntry, StoreError> {
        self.add_file_inner(src, name, None)
    }

    /// Ingest `src` under `name`, refusing (and storing nothing) unless its
    /// blake3 matches `expected` — the path `pull` uses for pinned downloads.
    pub fn add_file_verified(
        &self,
        src: &Path,
        name: &str,
        expected: &str,
    ) -> Result<RefEntry, StoreError> {
        self.add_file_inner(src, name, Some(expected))
    }

    fn add_file_inner(
        &self,
        src: &Path,
        name: &str,
        expected: Option<&str>,
    ) -> Result<RefEntry, StoreError> {
        let hash = Self::hash_file(src)?;
        if let Some(expected) = expected
            && expected != hash
        {
            return Err(StoreError::HashMismatch {
                path: src.to_path_buf(),
                expected: expected.to_string(),
                actual: hash,
            });
        }

        let ref_path = self.refs_dir().join(name);
        if ref_path.exists() {
            return Err(StoreError::RefExists(name.to_string()));
        }

        let blob_path = self.blobs_dir().join(&hash);
        if !blob_path.exists() {
            // Copy to tmp inside the store (same filesystem), then atomically
            // rename into place so a crashed ingest never leaves a torn blob.
            let tmp = self.tmp_dir().join(format!("ingest-{hash}"));
            fs::copy(src, &tmp).map_err(|e| io_err(&tmp, e))?;
            fs::rename(&tmp, &blob_path).map_err(|e| io_err(&blob_path, e))?;
            // Model weights aren't secret, and the DynamicUser
            // lisa-inferenced (not in group `lisa`) reads them via the world
            // bit — so force 0644 rather than depend on the downloader's
            // umask (a hardened 0077 would produce 0600 blobs the daemon
            // can't load, and inference would silently fail).
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&blob_path, fs::Permissions::from_mode(0o644))
                    .map_err(|e| io_err(&blob_path, e))?;
            }
        }
        fs::hard_link(&blob_path, &ref_path).map_err(|e| io_err(&ref_path, e))?;

        let size = fs::metadata(&blob_path)
            .map_err(|e| io_err(&blob_path, e))?
            .len();
        Ok(RefEntry {
            name: name.to_string(),
            blake3: hash,
            size,
        })
    }

    pub fn list(&self) -> Result<Vec<RefEntry>, StoreError> {
        // Refs are hardlinks, so recover each ref's blob hash by matching
        // inode numbers against the blobs directory.
        let mut ino_to_hash: HashMap<u64, String> = HashMap::new();
        let blobs = self.blobs_dir();
        for entry in fs::read_dir(&blobs).map_err(|e| io_err(&blobs, e))? {
            let entry = entry.map_err(|e| io_err(&blobs, e))?;
            let meta = entry.metadata().map_err(|e| io_err(&entry.path(), e))?;
            ino_to_hash.insert(meta.ino(), entry.file_name().to_string_lossy().into_owned());
        }

        let mut out = Vec::new();
        let refs = self.refs_dir();
        for entry in fs::read_dir(&refs).map_err(|e| io_err(&refs, e))? {
            let entry = entry.map_err(|e| io_err(&refs, e))?;
            let meta = entry.metadata().map_err(|e| io_err(&entry.path(), e))?;
            out.push(RefEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                blake3: ino_to_hash
                    .get(&meta.ino())
                    .cloned()
                    .unwrap_or_else(|| "<orphaned>".to_string()),
                size: meta.len(),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Recompute every blob's hash and compare against its filename.
    pub fn verify(&self) -> Result<VerifyReport, StoreError> {
        let mut report = VerifyReport::default();
        let blobs = self.blobs_dir();
        for entry in fs::read_dir(&blobs).map_err(|e| io_err(&blobs, e))? {
            let entry = entry.map_err(|e| io_err(&blobs, e))?;
            let expected = entry.file_name().to_string_lossy().into_owned();
            let actual = Self::hash_file(&entry.path())?;
            if expected == actual {
                report.ok += 1;
            } else {
                report.corrupt.push((entry.path(), expected, actual));
            }
        }
        Ok(report)
    }

    pub fn remove_ref(&self, name: &str) -> Result<(), StoreError> {
        let ref_path = self.refs_dir().join(name);
        if !ref_path.exists() {
            return Err(StoreError::RefNotFound(name.to_string()));
        }
        fs::remove_file(&ref_path).map_err(|e| io_err(&ref_path, e))
    }

    /// Remove blobs no ref points to (link count back down to 1). Returns
    /// the hashes removed.
    pub fn gc(&self) -> Result<Vec<String>, StoreError> {
        let mut removed = Vec::new();
        let blobs = self.blobs_dir();
        for entry in fs::read_dir(&blobs).map_err(|e| io_err(&blobs, e))? {
            let entry = entry.map_err(|e| io_err(&blobs, e))?;
            let meta = entry.metadata().map_err(|e| io_err(&entry.path(), e))?;
            if meta.nlink() == 1 {
                fs::remove_file(entry.path()).map_err(|e| io_err(&entry.path(), e))?;
                removed.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write_fixture(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(contents).unwrap();
        path
    }

    #[test]
    fn identical_content_shares_one_blob() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::open(tmp.path().join("store")).unwrap();
        let src = write_fixture(tmp.path(), "model.gguf", b"same-weights");

        let a = store.add_file(&src, "qwen3-8b-q4").unwrap();
        let b = store.add_file(&src, "qwen3-8b-q4-alias").unwrap();
        assert_eq!(a.blake3, b.blake3);

        let blob_count = fs::read_dir(store.root().join("blobs")).unwrap().count();
        assert_eq!(blob_count, 1, "identical content must dedupe to one blob");
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn verify_detects_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::open(tmp.path().join("store")).unwrap();
        let src = write_fixture(tmp.path(), "model.gguf", b"pristine");
        let entry = store.add_file(&src, "m").unwrap();

        assert!(store.verify().unwrap().is_clean());

        let blob = store.root().join("blobs").join(&entry.blake3);
        fs::write(&blob, b"tampered").unwrap();
        let report = store.verify().unwrap();
        assert_eq!(report.corrupt.len(), 1);
        assert_eq!(report.ok, 0);
    }

    #[test]
    fn pinned_ingest_rejects_wrong_hash_and_stores_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::open(tmp.path().join("store")).unwrap();
        let src = write_fixture(tmp.path(), "model.gguf", b"weights");

        let err = store
            .add_file_verified(&src, "m", &"0".repeat(64))
            .unwrap_err();
        assert!(matches!(err, StoreError::HashMismatch { .. }));
        assert_eq!(store.list().unwrap().len(), 0);
        let blob_count = fs::read_dir(store.root().join("blobs")).unwrap().count();
        assert_eq!(blob_count, 0);
    }

    #[test]
    fn gc_removes_only_unreferenced_blobs() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::open(tmp.path().join("store")).unwrap();
        let keep = write_fixture(tmp.path(), "keep.gguf", b"keep-me");
        let drop = write_fixture(tmp.path(), "drop.gguf", b"drop-me");
        store.add_file(&keep, "keep").unwrap();
        let dropped = store.add_file(&drop, "drop").unwrap();

        store.remove_ref("drop").unwrap();
        let removed = store.gc().unwrap();
        assert_eq!(removed, vec![dropped.blake3]);
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(store.verify().unwrap().is_clean());
    }

    #[test]
    fn duplicate_ref_name_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::open(tmp.path().join("store")).unwrap();
        let src = write_fixture(tmp.path(), "model.gguf", b"x");
        store.add_file(&src, "m").unwrap();
        assert!(matches!(
            store.add_file(&src, "m").unwrap_err(),
            StoreError::RefExists(_)
        ));
    }
}
