//! Per-app durable memory (`docs/PLAN.md` §5.3): namespace per app-id,
//! KV now, private vector collections with the embedding pipeline.
//! Isolation is API-shaped: every operation takes the app_id and no
//! query can cross it — the ACL fuzz suite (§5.3 acceptance) hammers
//! this boundary. Wipe is total: uninstall offers it, Settings exposes
//! it, and zero residual rows is the acceptance bar.

use crate::store::{ContextStore, StoreError};
use std::time::{SystemTime, UNIX_EPOCH};

impl ContextStore {
    pub fn memory_set(&self, app_id: &str, key: &str, value: &str) -> Result<(), StoreError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let conn = self.conn.lock().expect("context lock");
        conn.execute(
            "INSERT INTO app_memory (app_id, key, value, updated)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(app_id, key) DO UPDATE SET value = ?3, updated = ?4",
            rusqlite::params![app_id, key, value, now],
        )?;
        Ok(())
    }

    pub fn memory_get(&self, app_id: &str, key: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().expect("context lock");
        Ok(conn
            .query_row(
                "SELECT value FROM app_memory WHERE app_id = ?1 AND key = ?2",
                [app_id, key],
                |r| r.get(0),
            )
            .ok())
    }

    pub fn memory_list(&self, app_id: &str) -> Result<Vec<(String, String)>, StoreError> {
        let conn = self.conn.lock().expect("context lock");
        let mut stmt =
            conn.prepare("SELECT key, value FROM app_memory WHERE app_id = ?1 ORDER BY key")?;
        let rows = stmt.query_map([app_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Wipe an app's namespace entirely; returns rows removed. The §5.3
    /// acceptance bar is zero residual rows, verified by direct DB
    /// inspection — which this store invites (plain SQLite).
    pub fn memory_wipe(&self, app_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn.lock().expect("context lock");
        Ok(conn.execute("DELETE FROM app_memory WHERE app_id = ?1", [app_id])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_are_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        store.memory_set("org.app.a", "theme", "dark").unwrap();
        store.memory_set("org.app.b", "theme", "light").unwrap();

        assert_eq!(
            store.memory_get("org.app.a", "theme").unwrap().as_deref(),
            Some("dark")
        );
        assert_eq!(
            store.memory_get("org.app.b", "theme").unwrap().as_deref(),
            Some("light")
        );
        assert_eq!(store.memory_list("org.app.a").unwrap().len(), 1);
    }

    #[test]
    fn wipe_leaves_zero_residual_rows_for_that_app_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        for i in 0..5 {
            store
                .memory_set("org.app.a", &format!("k{i}"), "v")
                .unwrap();
        }
        store.memory_set("org.app.b", "survives", "yes").unwrap();

        assert_eq!(store.memory_wipe("org.app.a").unwrap(), 5);
        assert!(store.memory_list("org.app.a").unwrap().is_empty());
        assert_eq!(store.memory_list("org.app.b").unwrap().len(), 1);

        // Direct DB inspection, per the acceptance bar.
        let conn = store.conn.lock().unwrap();
        let residual: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM app_memory WHERE app_id = 'org.app.a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(residual, 0);
    }

    #[test]
    fn upsert_updates_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        store.memory_set("a", "k", "v1").unwrap();
        store.memory_set("a", "k", "v2").unwrap();
        assert_eq!(store.memory_get("a", "k").unwrap().as_deref(), Some("v2"));
        assert_eq!(store.memory_list("a").unwrap().len(), 1);
    }
}
