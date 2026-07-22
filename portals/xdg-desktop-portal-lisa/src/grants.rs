//! The grant store (`docs/PLAN.md` §5.5): per-app, per-scope consent
//! decisions, persisted as an **append-only action log** (the same
//! philosophy as the Ledger — state is derived, history is never
//! rewritten). Also owns the persisted half of quota accounting
//! (tokens/day), so a portal restart cannot reset an app's daily budget.
//!
//! Effective state = the last *persistent* action for (app, scope):
//! `allow` → allowed, `deny` → denied, `revoke` → back to unset (the
//! next request prompts again). `allow_once` is logged for the Ledger's
//! usage counts but never changes effective state.

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GrantError {
    #[error("grant store unavailable: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("grant store unavailable: {0}")]
    Io(#[from] std::io::Error),
}

/// One recorded consent action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantAction {
    Allow,
    AllowOnce,
    Deny,
    Revoke,
}

impl GrantAction {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantAction::Allow => "allow",
            GrantAction::AllowOnce => "allow_once",
            GrantAction::Deny => "deny",
            GrantAction::Revoke => "revoke",
        }
    }
}

/// Effective grant state derived from the action log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effective {
    Allowed,
    Denied,
    /// No persistent decision — first use (or post-revoke): prompt.
    Unset,
}

impl Effective {
    pub fn as_str(self) -> &'static str {
        match self {
            Effective::Allowed => "allowed",
            Effective::Denied => "denied",
            Effective::Unset => "unset",
        }
    }
}

/// A (app, scope) pair with its current effective state, for the
/// Settings › Intelligence panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRow {
    pub app_id: String,
    pub scope: String,
    pub state: Effective,
}

pub struct GrantStore {
    conn: Mutex<Connection>,
}

impl GrantStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, GrantError> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS grant_actions (
                id     INTEGER PRIMARY KEY AUTOINCREMENT,
                ts     INTEGER NOT NULL,
                app_id TEXT NOT NULL,
                scope  TEXT NOT NULL,
                action TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS grant_actions_app
                ON grant_actions(app_id, scope);
            CREATE TRIGGER IF NOT EXISTS grant_actions_no_update
                BEFORE UPDATE ON grant_actions
                BEGIN SELECT RAISE(ABORT, 'the grant log is append-only'); END;
            CREATE TRIGGER IF NOT EXISTS grant_actions_no_delete
                BEFORE DELETE ON grant_actions
                BEGIN SELECT RAISE(ABORT, 'the grant log is append-only'); END;
            CREATE TABLE IF NOT EXISTS quota_usage (
                app_id TEXT NOT NULL,
                day    TEXT NOT NULL,
                tokens INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (app_id, day)
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// In-memory store for tests and `--grants-db :memory:` dev runs.
    pub fn open_in_memory() -> Result<Self, GrantError> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        {
            let conn = store.conn.lock().expect("grant store lock");
            conn.execute_batch(
                "CREATE TABLE grant_actions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL,
                    app_id TEXT NOT NULL, scope TEXT NOT NULL, action TEXT NOT NULL);
                CREATE TABLE quota_usage (
                    app_id TEXT NOT NULL, day TEXT NOT NULL,
                    tokens INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (app_id, day));",
            )?;
        }
        Ok(store)
    }

    /// Default per-user location. The portal is a session service: its
    /// state is per-user by construction (multi-user must keep working,
    /// PLAN Appendix E).
    pub fn default_path() -> PathBuf {
        if let Some(p) = std::env::var_os("LISA_GRANTS_DB") {
            return PathBuf::from(p);
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|h| h.join(".local/share/lisa/grants.db"))
            .unwrap_or_else(|| PathBuf::from("lisa-grants.db"))
    }

    /// Append a consent action; returns its log id.
    pub fn record(
        &self,
        app_id: &str,
        scope: &str,
        action: GrantAction,
    ) -> Result<i64, GrantError> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let conn = self.conn.lock().expect("grant store lock");
        conn.execute(
            "INSERT INTO grant_actions (ts, app_id, scope, action) VALUES (?1,?2,?3,?4)",
            rusqlite::params![ts, app_id, scope, action.as_str()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The current effective state for (app, scope).
    pub fn effective(&self, app_id: &str, scope: &str) -> Result<Effective, GrantError> {
        let conn = self.conn.lock().expect("grant store lock");
        let last: Option<String> = conn
            .query_row(
                "SELECT action FROM grant_actions
                 WHERE app_id = ?1 AND scope = ?2 AND action != 'allow_once'
                 ORDER BY id DESC LIMIT 1",
                rusqlite::params![app_id, scope],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(match last.as_deref() {
            Some("allow") => Effective::Allowed,
            Some("deny") => Effective::Denied,
            _ => Effective::Unset,
        })
    }

    /// Every (app, scope) pair that ever asked, with its current state.
    pub fn list(&self) -> Result<Vec<GrantRow>, GrantError> {
        let pairs: Vec<(String, String)> = {
            let conn = self.conn.lock().expect("grant store lock");
            let mut stmt = conn.prepare(
                "SELECT DISTINCT app_id, scope FROM grant_actions ORDER BY app_id, scope",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        pairs
            .into_iter()
            .map(|(app_id, scope)| {
                let state = self.effective(&app_id, &scope)?;
                Ok(GrantRow {
                    app_id,
                    scope,
                    state,
                })
            })
            .collect()
    }

    /// Tokens consumed by `app_id` on `day` (day key: caller-supplied,
    /// e.g. "day-19923" — see [`crate::quota::day_key`]).
    pub fn tokens_used(&self, app_id: &str, day: &str) -> Result<i64, GrantError> {
        let conn = self.conn.lock().expect("grant store lock");
        conn.query_row(
            "SELECT tokens FROM quota_usage WHERE app_id = ?1 AND day = ?2",
            rusqlite::params![app_id, day],
            |r| r.get(0),
        )
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(0),
            other => Err(other.into()),
        })
    }

    /// Add `tokens` to the app's daily usage counter.
    pub fn add_tokens(&self, app_id: &str, day: &str, tokens: i64) -> Result<(), GrantError> {
        let conn = self.conn.lock().expect("grant store lock");
        conn.execute(
            "INSERT INTO quota_usage (app_id, day, tokens) VALUES (?1, ?2, ?3)
             ON CONFLICT(app_id, day) DO UPDATE SET tokens = tokens + ?3",
            rusqlite::params![app_id, day, tokens],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> GrantStore {
        GrantStore::open_in_memory().unwrap()
    }

    #[test]
    fn first_use_is_unset_then_allow_sticks() {
        let s = store();
        assert_eq!(s.effective("app.a", "inference").unwrap(), Effective::Unset);
        s.record("app.a", "inference", GrantAction::Allow).unwrap();
        assert_eq!(
            s.effective("app.a", "inference").unwrap(),
            Effective::Allowed
        );
        // Another app never inherits the grant.
        assert_eq!(s.effective("app.b", "inference").unwrap(), Effective::Unset);
    }

    #[test]
    fn allow_once_never_persists() {
        let s = store();
        s.record("app.a", "inference", GrantAction::AllowOnce)
            .unwrap();
        assert_eq!(s.effective("app.a", "inference").unwrap(), Effective::Unset);
    }

    #[test]
    fn deny_sticks_and_revoke_resets_to_unset() {
        let s = store();
        s.record("app.a", "inference", GrantAction::Deny).unwrap();
        assert_eq!(
            s.effective("app.a", "inference").unwrap(),
            Effective::Denied
        );
        s.record("app.a", "inference", GrantAction::Allow).unwrap();
        assert_eq!(
            s.effective("app.a", "inference").unwrap(),
            Effective::Allowed
        );
        s.record("app.a", "inference", GrantAction::Revoke).unwrap();
        assert_eq!(s.effective("app.a", "inference").unwrap(), Effective::Unset);
    }

    #[test]
    fn list_reports_current_state_per_pair() {
        let s = store();
        s.record("app.a", "inference", GrantAction::Allow).unwrap();
        s.record("app.b", "inference", GrantAction::Deny).unwrap();
        let rows = s.list().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].app_id, "app.a");
        assert_eq!(rows[0].state, Effective::Allowed);
        assert_eq!(rows[1].state, Effective::Denied);
    }

    #[test]
    fn grant_log_is_append_only_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let s = GrantStore::open(dir.path().join("grants.db")).unwrap();
        s.record("app.a", "inference", GrantAction::Allow).unwrap();
        let conn = s.conn.lock().unwrap();
        assert!(
            conn.execute("UPDATE grant_actions SET action='deny'", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM grant_actions", []).is_err());
    }

    #[test]
    fn daily_token_accounting_accumulates_per_app_per_day() {
        let s = store();
        assert_eq!(s.tokens_used("app.a", "day-1").unwrap(), 0);
        s.add_tokens("app.a", "day-1", 100).unwrap();
        s.add_tokens("app.a", "day-1", 50).unwrap();
        s.add_tokens("app.a", "day-2", 7).unwrap();
        assert_eq!(s.tokens_used("app.a", "day-1").unwrap(), 150);
        assert_eq!(s.tokens_used("app.a", "day-2").unwrap(), 7);
        assert_eq!(s.tokens_used("app.b", "day-1").unwrap(), 0);
    }
}
