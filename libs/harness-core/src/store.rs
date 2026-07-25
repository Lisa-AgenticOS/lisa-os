//! The storage seam for KV-persisted pillars. [`KvStore`] mirrors the
//! `dev.lisaos.Context1` app-memory verbs (`MemoryGet`/`MemorySet`,
//! PLAN §5.3) *within one app's namespace* — the same surface the
//! Assistant already persists its conversation through. harness-core
//! stays IPC-free (no D-Bus here): on Lisa the caller implements this
//! trait over a `Context1` proxy; tests and non-persistent embedders
//! use [`MemKv`].
//!
//! `Context1` has no per-key delete — only `MemoryWipe` for the whole
//! namespace — so [`KvStore::remove`]'s default implementation
//! tombstones the key with the empty string, and readers treat an empty
//! value as absent. Backends with real deletion (like [`MemKv`])
//! override it.

use crate::Error;
use std::collections::HashMap;
use std::sync::Mutex;

/// One app-memory namespace: get/set string values by key. A `Context1`
/// bridge maps `MemoryGet`'s missing-key error to `Ok(None)`.
pub trait KvStore {
    fn get(&self, key: &str) -> Result<Option<String>, Error>;
    fn set(&self, key: &str, value: &str) -> Result<(), Error>;

    /// Remove `key`. Default: tombstone with the empty string, because
    /// the `Context1` substrate cannot delete individual keys.
    fn remove(&self, key: &str) -> Result<(), Error> {
        self.set(key, "")
    }
}

impl<T: KvStore + ?Sized> KvStore for &T {
    fn get(&self, key: &str) -> Result<Option<String>, Error> {
        (**self).get(key)
    }

    fn set(&self, key: &str, value: &str) -> Result<(), Error> {
        (**self).set(key, value)
    }

    fn remove(&self, key: &str) -> Result<(), Error> {
        (**self).remove(key)
    }
}

/// In-memory [`KvStore`] — the test double, and the store for embedders
/// that don't want persistence.
#[derive(Debug, Default)]
pub struct MemKv {
    map: Mutex<HashMap<String, String>>,
}

impl KvStore for MemKv {
    fn get(&self, key: &str) -> Result<Option<String>, Error> {
        Ok(self.map.lock().expect("kv lock").get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), Error> {
        self.map
            .lock()
            .expect("kv lock")
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), Error> {
        self.map.lock().expect("kv lock").remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memkv_round_trip_and_real_removal() {
        let kv = MemKv::default();
        assert_eq!(kv.get("missing").unwrap(), None);
        kv.set("k", "v1").unwrap();
        kv.set("k", "v2").unwrap();
        assert_eq!(kv.get("k").unwrap().as_deref(), Some("v2"));
        kv.remove("k").unwrap();
        assert_eq!(kv.get("k").unwrap(), None, "MemKv deletes for real");
    }

    #[test]
    fn default_remove_tombstones_for_substrates_without_delete() {
        // A Context1-shaped store: only get/set, remove left at default.
        struct GetSetOnly(MemKv);
        impl KvStore for GetSetOnly {
            fn get(&self, key: &str) -> Result<Option<String>, Error> {
                self.0.get(key)
            }
            fn set(&self, key: &str, value: &str) -> Result<(), Error> {
                self.0.set(key, value)
            }
        }
        let kv = GetSetOnly(MemKv::default());
        kv.set("k", "v").unwrap();
        kv.remove("k").unwrap();
        assert_eq!(
            kv.get("k").unwrap().as_deref(),
            Some(""),
            "tombstoned, not deleted — readers treat empty as absent"
        );
    }

    #[test]
    fn references_are_stores_too() {
        fn takes_store(kv: impl KvStore) {
            kv.set("k", "v").unwrap();
        }
        let kv = MemKv::default();
        takes_store(&kv);
        assert_eq!(kv.get("k").unwrap().as_deref(), Some("v"));
    }
}
