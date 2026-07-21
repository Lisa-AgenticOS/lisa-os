//! The Lisa Ledger — append-only audit log (`docs/PLAN.md` §4 rule 4,
//! §5.7.6, §5.10).
//!
//! Radical legibility as a mechanism, not a promise: every model call,
//! context grant, and tool execution lands here *before* it happens —
//! no ledger entry, no inference. Append-only is enforced in the schema
//! itself: SQLite triggers abort every UPDATE and DELETE, and the file
//! is plain SQLite the user can open with `sqlite3`.
//!
//! M2 attaches per-app identity (portal) and the prompt envelope
//! (context chunks + provenance); the Ledger app (§5.7.6) renders it.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("ledger unavailable: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("ledger unavailable: {0}")]
    Io(#[from] std::io::Error),
}

/// One auditable event. `kind` examples: `inference.generate`,
/// `inference.embed`, `inference.complete`, `context.grant`, `tool.call`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: i64,
    /// Unix milliseconds.
    pub ts: i64,
    pub kind: String,
    /// Requesting app identity; "host" until the portal attaches real
    /// per-app identity (M2).
    pub app_id: String,
    pub model: String,
    /// blake3 of the full prompt/input (the input itself may be large).
    pub input_hash: String,
    /// Human-readable preview for the Ledger UI (bounded).
    pub preview: String,
    /// "ok" | "error" | "preempted" | "started"
    pub status: String,
    pub detail: String,
    /// For completion entries: the id of the corresponding start entry.
    pub ref_id: Option<i64>,
    pub output_tokens: i64,
    pub duration_ms: i64,
}

/// What callers provide when appending (ids/timestamps are the ledger's).
#[derive(Debug, Clone, Default)]
pub struct Event {
    pub kind: String,
    pub app_id: String,
    pub model: String,
    pub input_hash: String,
    pub preview: String,
    pub status: String,
    pub detail: String,
    pub ref_id: Option<i64>,
    pub output_tokens: i64,
    pub duration_ms: i64,
}

pub struct Ledger {
    conn: Mutex<Connection>,
}

impl Ledger {
    /// Open (creating if needed) the ledger at `path`. The schema is
    /// append-only by construction: triggers abort UPDATE and DELETE.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LedgerError> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                ts            INTEGER NOT NULL,
                kind          TEXT NOT NULL,
                app_id        TEXT NOT NULL,
                model         TEXT NOT NULL,
                input_hash    TEXT NOT NULL,
                preview       TEXT NOT NULL,
                status        TEXT NOT NULL,
                detail        TEXT NOT NULL,
                ref_id        INTEGER,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                duration_ms   INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS entries_ts ON entries(ts);
            CREATE INDEX IF NOT EXISTS entries_kind ON entries(kind);
            CREATE TRIGGER IF NOT EXISTS ledger_no_update
                BEFORE UPDATE ON entries
                BEGIN SELECT RAISE(ABORT, 'the ledger is append-only'); END;
            CREATE TRIGGER IF NOT EXISTS ledger_no_delete
                BEFORE DELETE ON entries
                BEGIN SELECT RAISE(ABORT, 'the ledger is append-only'); END;",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Default location, in order: systemd's STATE_DIRECTORY (the
    /// hardened unit passes StateDirectory=lisa), /var/lib/lisa when it
    /// exists (image / layer installs), else per-user under
    /// ~/.local/share/lisa.
    pub fn default_path() -> PathBuf {
        if let Some(state) = std::env::var_os("STATE_DIRECTORY") {
            return PathBuf::from(state).join("ledger.db");
        }
        let system = PathBuf::from("/var/lib/lisa");
        if system.is_dir() {
            return system.join("ledger.db");
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|h| h.join(".local/share/lisa/ledger.db"))
            .unwrap_or_else(|| system.join("ledger.db"))
    }

    /// Append an event; returns its ledger id. This is the gate other
    /// components rely on: if this fails, the action must not happen.
    pub fn append(&self, e: &Event) -> Result<i64, LedgerError> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let conn = self.conn.lock().expect("ledger lock");
        conn.execute(
            "INSERT INTO entries
               (ts, kind, app_id, model, input_hash, preview, status, detail,
                ref_id, output_tokens, duration_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            rusqlite::params![
                ts,
                e.kind,
                e.app_id,
                e.model,
                e.input_hash,
                e.preview,
                e.status,
                e.detail,
                e.ref_id,
                e.output_tokens,
                e.duration_ms,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Most recent `limit` entries, newest first.
    pub fn tail(&self, limit: usize) -> Result<Vec<Entry>, LedgerError> {
        let conn = self.conn.lock().expect("ledger lock");
        let mut stmt = conn.prepare(
            "SELECT id, ts, kind, app_id, model, input_hash, preview, status,
                    detail, ref_id, output_tokens, duration_ms
             FROM entries ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(Entry {
                id: r.get(0)?,
                ts: r.get(1)?,
                kind: r.get(2)?,
                app_id: r.get(3)?,
                model: r.get(4)?,
                input_hash: r.get(5)?,
                preview: r.get(6)?,
                status: r.get(7)?,
                detail: r.get(8)?,
                ref_id: r.get(9)?,
                output_tokens: r.get(10)?,
                duration_ms: r.get(11)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn count(&self) -> Result<i64, LedgerError> {
        let conn = self.conn.lock().expect("ledger lock");
        Ok(conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?)
    }
}

/// Bounded, single-line preview for UI display.
pub fn preview_of(text: &str) -> String {
    let one_line = text.replace(['\n', '\r'], " ");
    one_line.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ledger() -> (tempfile::TempDir, Ledger) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(dir.path().join("ledger.db")).unwrap();
        (dir, ledger)
    }

    fn event(kind: &str) -> Event {
        Event {
            kind: kind.into(),
            app_id: "host".into(),
            model: "test".into(),
            input_hash: "abc".into(),
            preview: "hello".into(),
            status: "started".into(),
            ..Default::default()
        }
    }

    #[test]
    fn append_and_tail_round_trip() {
        let (_dir, ledger) = test_ledger();
        let a = ledger.append(&event("inference.generate")).unwrap();
        let b = ledger
            .append(&Event {
                kind: "inference.complete".into(),
                ref_id: Some(a),
                status: "ok".into(),
                output_tokens: 42,
                ..event("inference.complete")
            })
            .unwrap();
        assert!(b > a);
        let tail = ledger.tail(10).unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].id, b, "newest first");
        assert_eq!(tail[0].ref_id, Some(a));
        assert_eq!(ledger.count().unwrap(), 2);
    }

    #[test]
    fn update_and_delete_are_impossible() {
        let (_dir, ledger) = test_ledger();
        ledger.append(&event("inference.generate")).unwrap();
        let conn = ledger.conn.lock().unwrap();
        let update = conn.execute("UPDATE entries SET status='tampered'", []);
        assert!(update.is_err(), "UPDATE must be rejected");
        let delete = conn.execute("DELETE FROM entries", []);
        assert!(delete.is_err(), "DELETE must be rejected");
    }

    #[test]
    fn open_on_unwritable_path_fails() {
        assert!(Ledger::open("/proc/definitely/not/writable/ledger.db").is_err());
    }

    #[test]
    fn preview_is_bounded_and_single_line() {
        let p = preview_of(&format!("line1\nline2 {}", "x".repeat(500)));
        assert!(p.len() <= 160);
        assert!(!p.contains('\n'));
    }
}
