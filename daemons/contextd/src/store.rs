//! The context store: one SQLite file per user, FTS5 for lexical search
//! (`docs/PLAN.md` §5.3). Encryption-at-rest arrives with the keyring
//! integration; vectors arrive with the embedding pipeline.

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("context store: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("context store io: {0}")]
    Io(#[from] std::io::Error),
}

pub struct ContextStore {
    pub(crate) conn: Mutex<Connection>,
}

impl ContextStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS documents (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                source    TEXT NOT NULL,           -- absolute path / URI
                provenance TEXT NOT NULL,          -- file | mail | screen | web
                mtime     INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                UNIQUE(source)
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS chunks USING fts5(
                content,
                doc_id UNINDEXED,
                seq UNINDEXED
            );
            CREATE TABLE IF NOT EXISTS app_memory (
                app_id  TEXT NOT NULL,
                key     TEXT NOT NULL,
                value   TEXT NOT NULL,
                updated INTEGER NOT NULL,
                PRIMARY KEY (app_id, key)
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Per-user default: ~/.local/share/lisa/context/context.db.
    pub fn default_path() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|h| h.join(".local/share/lisa/context/context.db"))
            .unwrap_or_else(|| PathBuf::from("/var/lib/lisa/context.db"))
    }
}
