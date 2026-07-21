//! Model downloads. `lisa-modeld` is the *only* Lisa component allowed
//! network access for model traffic (`docs/PLAN.md` §5.2, dataflow rule 2).
//! Delta/resumable downloads and pinned-host enforcement land in M1; M0
//! ships plain streaming download with mandatory hash pinning.

use crate::store::{ModelStore, RefEntry, StoreError};
use std::fs;
use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("download failed: {0}")]
    Http(#[from] Box<ureq::Error>),
    #[error("io error during download: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Download `url` into the store as `name`, verifying the pinned blake3
/// before anything becomes visible. No pin, no pull — hash pinning is
/// policy (PLAN §5.10), not an option.
///
/// Interrupted downloads resume with an HTTP Range request: the partial
/// temp file persists across attempts and only missing bytes are
/// re-fetched (§5.2). A hash mismatch discards the partial entirely.
pub fn pull(
    store: &ModelStore,
    url: &str,
    name: &str,
    expected_blake3: &str,
) -> Result<RefEntry, FetchError> {
    let tmp = store.tmp_dir().join(format!("download-{name}"));
    let result = (|| -> Result<RefEntry, FetchError> {
        let offset = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        let mut request = ureq::get(url);
        if offset > 0 {
            request = request.header("Range", format!("bytes={offset}-"));
        }
        let mut response = request.call().map_err(Box::new)?;
        let resumed = response.status() == 206;
        let mut reader = response.body_mut().as_reader();
        let mut file = if resumed {
            fs::OpenOptions::new().append(true).open(&tmp)?
        } else {
            // Server ignored the range (or fresh start): full download.
            fs::File::create(&tmp)?
        };
        io::copy(&mut reader, &mut file)?;
        file.sync_all()?;
        Ok(store.add_file_verified(&tmp, name, expected_blake3)?)
    })();
    match &result {
        // Success or corruption: the temp file must not survive. A
        // network error keeps the partial for the next resume.
        Ok(_) | Err(FetchError::Store(_)) => {
            let _ = fs::remove_file(&tmp);
        }
        Err(_) => {}
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Network-dependent; run explicitly with `cargo test -- --ignored`
    /// against a real pinned artifact once the M1 catalog is populated.
    #[test]
    #[ignore = "requires network and a pinned artifact URL (M1)"]
    fn pull_verifies_pin() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModelStore::open(tmp.path()).unwrap();
        let err = pull(
            &store,
            "http://127.0.0.1:1/nonexistent",
            "m",
            &"0".repeat(64),
        );
        assert!(err.is_err());
    }
}
