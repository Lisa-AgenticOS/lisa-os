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
    #[error(
        "{0:?} is not a provenance this store knows — a document written \
         under an unknown tag is readable by no scope at all, which looks \
         like an empty index rather than a mistake"
    )]
    UnknownProvenance(String),
    // NOT a field called `source`: thiserror reads that name as the
    // error's cause and demands it implement Error.
    #[error(
        "{doc_source:?} is already indexed as {existing:?}; refusing to \
         relabel it {new:?}"
    )]
    ProvenanceConflict {
        doc_source: String,
        existing: String,
        new: String,
    },
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
        // Two session-start units (contextd itself and
        // lisa-knowledge-sync) open this store unordered, and since the
        // porter migration (#176) open() can be a WRITER. Without a
        // busy timeout the loser of that race gets an immediate
        // SQLITE_BUSY on exactly the first boot after an upgrade —
        // found in review (#179) before it found a device.
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS documents (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                source    TEXT NOT NULL,           -- absolute path / URI
                provenance TEXT NOT NULL,          -- file | mail | screen | web
                mtime     INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                UNIQUE(source)
            );
            -- porter: 'invoice' must match 'invoices'. Reproduced on the
            -- reference device (#176): an indexed, embedded document
            -- containing 'quarterly invoices' returned NOTHING for the
            -- query 'quarterly invoice', because FTS5's default
            -- tokenizer does no stemming and MATCH terms are ANDed.
            CREATE VIRTUAL TABLE IF NOT EXISTS chunks USING fts5(
                content,
                doc_id UNINDEXED,
                seq UNINDEXED,
                tokenize = 'porter unicode61'
            );
            CREATE TABLE IF NOT EXISTS app_memory (
                app_id  TEXT NOT NULL,
                key     TEXT NOT NULL,
                value   TEXT NOT NULL,
                updated INTEGER NOT NULL,
                PRIMARY KEY (app_id, key)
            );
            -- Per-chunk embeddings for hybrid retrieval (§5.3). Stored as
            -- little-endian f32 blobs; brute-force cosine over FTS5-
            -- prefiltered candidates for now (sqlite-vec is the later
            -- optimization). Keyed to a chunk's (doc_id, seq).
            CREATE TABLE IF NOT EXISTS chunk_vectors (
                doc_id INTEGER NOT NULL,
                seq    INTEGER NOT NULL,
                dim    INTEGER NOT NULL,
                vec    BLOB NOT NULL,
                PRIMARY KEY (doc_id, seq)
            );",
        )?;

        // Migrate a pre-porter chunks table in place (#176). An FTS5
        // tokenizer is fixed at CREATE time, so an existing store keeps
        // its old one silently — same content, worse recall, nothing
        // logged — unless rebuilt. The rebuild copies (content, doc_id,
        // seq) verbatim, so chunk_vectors rows keyed on (doc_id, seq)
        // stay valid and nothing needs re-embedding.
        let schema: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'chunks'",
                [],
                |r| r.get(0),
            )
            .unwrap_or_default();
        if !schema.contains("porter") {
            conn.execute_batch(
                "BEGIN;
                 CREATE VIRTUAL TABLE chunks_porter USING fts5(
                     content,
                     doc_id UNINDEXED,
                     seq UNINDEXED,
                     tokenize = 'porter unicode61'
                 );
                 INSERT INTO chunks_porter (content, doc_id, seq)
                     SELECT content, doc_id, seq FROM chunks;
                 DROP TABLE chunks;
                 ALTER TABLE chunks_porter RENAME TO chunks;
                 COMMIT;",
            )?;
        }

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
