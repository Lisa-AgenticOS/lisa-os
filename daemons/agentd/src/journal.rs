//! The undo journal (`docs/PLAN.md` §5.4): every executed write-tier
//! action records a compensation — the app-provided inverse call from
//! its manifest's `undo` declaration — so `lisa undo` (and the shell
//! affordance) can revert the last agent action.
//!
//! The journal is *not* the Ledger: the Ledger is the append-only audit
//! trail; the journal is working state (entries move active → undone /
//! skipped). Actions whose compensation could not be resolved are still
//! recorded, so undo can honestly say "not undoable" instead of
//! silently reverting an older action. The btrfs-snapshot compensation
//! lane for file ops arrives with the file tools (ADR-0009).

use crate::manifest::UndoDecl;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("undo journal unavailable: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("undo journal unavailable: {0}")]
    Io(#[from] std::io::Error),
}

/// One journaled action. `state`: "active" | "undone" | "skipped".
#[derive(Debug, Clone, Serialize)]
pub struct JournalEntry {
    pub id: i64,
    /// Unix milliseconds.
    pub ts: i64,
    /// Ledger id of the `tool.call` entry this action belongs to.
    pub ledger_ref: i64,
    /// Free-text label for the Ledger ("cli", "shell"). It is ASSERTED
    /// by the caller and authorises nothing — see `owner`.
    pub actor: String,
    /// The transport-assigned peer that made this action (ADR-0033).
    /// This is what `undo` checks. `None` on rows written before the
    /// column existed; those are undoable by nobody, which is the
    /// fail-closed direction (#94).
    pub owner: Option<String>,
    pub app_id: String,
    pub tool: String,
    pub args_json: String,
    pub result_json: String,
    pub undo_tool: Option<String>,
    pub undo_args_json: Option<String>,
    pub state: String,
}

/// One action to journal; `undo` is the resolved compensation call
/// (tool + args), or None when the action declares no inverse.
pub struct NewRecord<'a> {
    /// Ledger id of the `tool.call` entry this action belongs to.
    pub ledger_ref: i64,
    pub actor: &'a str,
    /// Transport-assigned identity of the peer that made this call.
    pub owner: &'a str,
    pub app_id: &'a str,
    pub tool: &'a str,
    pub args: &'a Value,
    pub result: &'a Value,
    pub undo: Option<(String, Value)>,
}

pub struct UndoJournal {
    conn: Mutex<Connection>,
}

impl UndoJournal {
    pub fn open(path: impl AsRef<Path>) -> Result<UndoJournal, JournalError> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::from_conn(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<UndoJournal, JournalError> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<UndoJournal, JournalError> {
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS journal (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                ts             INTEGER NOT NULL,
                ledger_ref     INTEGER NOT NULL,
                actor          TEXT NOT NULL,
                app_id         TEXT NOT NULL,
                tool           TEXT NOT NULL,
                args_json      TEXT NOT NULL,
                result_json    TEXT NOT NULL,
                undo_tool      TEXT,
                undo_args_json TEXT,
                state          TEXT NOT NULL DEFAULT 'active',
                owner          TEXT
            );
            CREATE INDEX IF NOT EXISTS journal_state ON journal(state, id);",
        )?;
        // Migration for databases created before `owner` existed (#94).
        // A duplicate-column error means it is already there, which is
        // the normal case on every boot after the first.
        let _ = conn.execute("ALTER TABLE journal ADD COLUMN owner TEXT", []);
        Ok(UndoJournal {
            conn: Mutex::new(conn),
        })
    }

    /// Default location, mirroring `lisa_ledger::Ledger::default_path`.
    pub fn default_path() -> PathBuf {
        lisa_ledger::Ledger::default_path().with_file_name("agent-journal.db")
    }

    /// Record an executed privileged action.
    pub fn record(&self, rec: NewRecord<'_>) -> Result<i64, JournalError> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let (undo_tool, undo_args) = match rec.undo {
            Some((t, a)) => (Some(t), Some(a.to_string())),
            None => (None, None),
        };
        let conn = self.conn.lock().expect("journal lock");
        conn.execute(
            "INSERT INTO journal
               (ts, ledger_ref, actor, app_id, tool, args_json, result_json,
                undo_tool, undo_args_json, state, owner)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'active',?10)",
            rusqlite::params![
                ts,
                rec.ledger_ref,
                rec.actor,
                rec.app_id,
                rec.tool,
                rec.args.to_string(),
                rec.result.to_string(),
                undo_tool,
                undo_args,
                rec.owner,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Newest still-active entry **belonging to `owner`** — the one
    /// `lisa undo` targets.
    ///
    /// Scoping by owner is the fix for #94. Undo dispatches a
    /// compensation that is frequently destructive-tier, and an unscoped
    /// query meant any peer on the session bus could revert an action
    /// some other peer had taken. Ownership is the right authority here
    /// rather than a fresh confirmation: undo reverts an action this
    /// peer *made*, and having made it is the permission. Asking again
    /// would make undo unusable while adding nothing.
    ///
    /// Rows written before the `owner` column existed have NULL and
    /// therefore match nobody. Losing the ability to undo pre-upgrade
    /// actions is the fail-closed direction, and the alternative —
    /// treating NULL as "anyone" — is the vulnerability itself.
    pub fn last_active(&self, owner: &str) -> Result<Option<JournalEntry>, JournalError> {
        let conn = self.conn.lock().expect("journal lock");
        let mut stmt = conn.prepare(
            "SELECT id, ts, ledger_ref, actor, app_id, tool, args_json,
                    result_json, undo_tool, undo_args_json, state, owner
             FROM journal WHERE state = 'active' AND owner = ?1
             ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map([owner], |r| {
            Ok(JournalEntry {
                id: r.get(0)?,
                ts: r.get(1)?,
                ledger_ref: r.get(2)?,
                actor: r.get(3)?,
                app_id: r.get(4)?,
                tool: r.get(5)?,
                args_json: r.get(6)?,
                result_json: r.get(7)?,
                undo_tool: r.get(8)?,
                undo_args_json: r.get(9)?,
                state: r.get(10)?,
                owner: r.get(11)?,
            })
        })?;
        rows.next().transpose().map_err(Into::into)
    }

    /// Move an entry out of the active stack ("undone" or "skipped").
    pub fn set_state(&self, id: i64, state: &str) -> Result<(), JournalError> {
        let conn = self.conn.lock().expect("journal lock");
        conn.execute(
            "UPDATE journal SET state = ?2 WHERE id = ?1",
            rusqlite::params![id, state],
        )?;
        Ok(())
    }
}

/// Build the compensation args for an executed call from its manifest
/// `undo.map`: values are literals, `$input`/`$result`, or dotted paths
/// into either (Appendix B: `{"event_id": "$result.event_id"}`).
pub fn resolve_undo(decl: &UndoDecl, input: &Value, result: &Value) -> Result<Value, String> {
    let mut args = serde_json::Map::new();
    for (key, spec) in &decl.map {
        let value = if let Some(path) = spec.strip_prefix("$result") {
            walk(result, path).ok_or_else(|| format!("{spec}: not present in result"))?
        } else if let Some(path) = spec.strip_prefix("$input") {
            walk(input, path).ok_or_else(|| format!("{spec}: not present in input"))?
        } else {
            Value::String(spec.clone())
        };
        args.insert(key.clone(), value);
    }
    Ok(Value::Object(args))
}

fn walk(root: &Value, dotted: &str) -> Option<Value> {
    let mut current = root;
    for seg in dotted.split('.').filter(|s| !s.is_empty()) {
        current = current.get(seg)?;
    }
    Some(current.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn journal() -> UndoJournal {
        UndoJournal::open_in_memory().unwrap()
    }

    fn record_add(j: &UndoJournal, ledger_ref: i64, undo: Option<(String, Value)>) -> i64 {
        j.record(NewRecord {
            ledger_ref,
            actor: "host",
            owner: "test-peer",
            app_id: "org.gnome.Calendar",
            tool: "add_event",
            args: &json!({"title": "dentist"}),
            result: &json!({"event_id": "evt-1"}),
            undo,
        })
        .unwrap()
    }

    #[test]
    fn last_active_is_newest_and_state_transitions_pop_the_stack() {
        let j = journal();
        let a = record_add(
            &j,
            10,
            Some(("delete_event".into(), json!({"event_id": "evt-1"}))),
        );
        let b = record_add(&j, 20, None);
        let top = j.last_active("test-peer").unwrap().unwrap();
        assert_eq!(top.id, b);
        assert!(
            top.undo_tool.is_none(),
            "not-undoable entries still journal"
        );

        j.set_state(b, "skipped").unwrap();
        let top = j.last_active("test-peer").unwrap().unwrap();
        assert_eq!(top.id, a);
        assert_eq!(top.undo_tool.as_deref(), Some("delete_event"));

        j.set_state(a, "undone").unwrap();
        assert!(j.last_active("test-peer").unwrap().is_none());
    }

    #[test]
    fn resolve_undo_maps_result_input_and_literals() {
        let decl = UndoDecl {
            tool: "delete_event".into(),
            map: BTreeMap::from([
                ("event_id".into(), "$result.event_id".into()),
                ("title".into(), "$input.title".into()),
                ("mode".into(), "hard".into()),
                ("everything".into(), "$result".into()),
            ]),
        };
        let input = json!({"title": "dentist", "start": "2026-07-23T10:00:00Z"});
        let result = json!({"event_id": "evt-9", "nested": {"x": 1}});
        let args = resolve_undo(&decl, &input, &result).unwrap();
        assert_eq!(args["event_id"], "evt-9");
        assert_eq!(args["title"], "dentist");
        assert_eq!(args["mode"], "hard");
        assert_eq!(args["everything"], result);
    }

    #[test]
    fn resolve_undo_fails_on_missing_path() {
        let decl = UndoDecl {
            tool: "delete_event".into(),
            map: BTreeMap::from([("event_id".into(), "$result.missing".into())]),
        };
        let err = resolve_undo(&decl, &json!({}), &json!({})).unwrap_err();
        assert!(err.contains("$result.missing"), "{err}");
    }

    #[test]
    fn default_path_sits_next_to_the_ledger() {
        let p = UndoJournal::default_path();
        assert_eq!(p.file_name().unwrap(), "agent-journal.db");
        assert_eq!(
            p.parent(),
            lisa_ledger::Ledger::default_path().parent(),
            "journal lives beside the ledger"
        );
    }
}
