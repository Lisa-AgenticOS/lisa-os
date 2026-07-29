//! The grant store (`docs/PLAN.md` §5.5): per-app, per-scope consent
//! decisions, persisted as an **append-only action log** (the same
//! philosophy as the Ledger — state is derived, history is never
//! rewritten). Also owns the persisted half of quota accounting
//! (tokens/day), so a portal restart cannot reset an app's daily budget.
//!
//! Effective state = the last *persistent* action for (app, scope):
//! `allow` → allowed, `deny` → denied, `revoke` → back to unset (the
//! next request prompts again). `allow_once` and `deny_once` are logged
//! — the first for the Ledger's usage counts, the second because a
//! refusal the system forgets instantly is what makes prompt-until-they-
//! click-yes work (issue #113) — but neither changes effective state.

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
    /// A refusal the user did not ask to remember — "no, not now", a
    /// dismissed dialog, or no dialog service at all. It does not change
    /// effective state, but it is *recorded*, which is what lets the
    /// portal notice an app asking again and again (#113).
    DenyOnce,
    Revoke,
}

impl GrantAction {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantAction::Allow => "allow",
            GrantAction::AllowOnce => "allow_once",
            GrantAction::Deny => "deny",
            GrantAction::DenyOnce => "deny_once",
            GrantAction::Revoke => "revoke",
        }
    }

    /// Whether this action decides the effective state. The transient
    /// ones are history, not policy.
    pub fn is_persistent(self) -> bool {
        !matches!(self, GrantAction::AllowOnce | GrantAction::DenyOnce)
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

/// The one schema, used by both constructors.
///
/// It was two, and they had drifted: the in-memory store had no
/// append-only triggers, so every test in the suite ran against a store
/// that permitted exactly what the on-disk one forbids. A second copy of
/// a schema is a second policy.
const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS grant_actions (
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
    );";

impl GrantStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, GrantError> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// In-memory store for tests and `--grants-db :memory:` dev runs.
    pub fn open_in_memory() -> Result<Self, GrantError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
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
        self.record_at(app_id, scope, action, now_ms())
    }

    /// Append with an explicit timestamp — the form the cooldown tests
    /// need, since "wait an hour" is not a test.
    pub fn record_at(
        &self,
        app_id: &str,
        scope: &str,
        action: GrantAction,
        ts: i64,
    ) -> Result<i64, GrantError> {
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
                 WHERE app_id = ?1 AND scope = ?2
                   AND action NOT IN ('allow_once', 'deny_once')
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

    /// How many times (app, scope) has been refused without the user
    /// asking to remember it, since `since_ms`.
    ///
    /// The counter resets on any persistent decision: once the user
    /// answers "always" or "never", earlier hesitation is not evidence
    /// about the app any more. Without that reset an app the user
    /// eventually allowed would still be carrying a cooldown.
    pub fn refusals_since(
        &self,
        app_id: &str,
        scope: &str,
        since_ms: i64,
    ) -> Result<u32, GrantError> {
        let conn = self.conn.lock().expect("grant store lock");
        let last_persistent: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(id), 0) FROM grant_actions
                 WHERE app_id = ?1 AND scope = ?2
                   AND action NOT IN ('allow_once', 'deny_once')",
                rusqlite::params![app_id, scope],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM grant_actions
             WHERE app_id = ?1 AND scope = ?2 AND action = 'deny_once'
               AND ts >= ?3 AND id > ?4",
            rusqlite::params![app_id, scope, since_ms, last_persistent],
            |r| r.get(0),
        )?;
        Ok(count as u32)
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

    /// Charge `tokens` against the app's daily budget, all or nothing.
    ///
    /// Returns `Ok(false)` — and writes nothing — when the request would
    /// take the app past `cap`. Two properties issue #114 found missing:
    ///
    /// 1. **The request must fit.** The old check was `used >= cap`
    ///    *before* adding, so any single request was admitted whole as
    ///    long as the counter had not already hit the cap: a 1000-token
    ///    call against a 5-token budget went through and left the
    ///    counter at 1000. A cap you can exceed by an unbounded amount
    ///    is a speed bump.
    /// 2. **Read and write are one transaction.** They were two lock
    ///    acquisitions, so concurrent requests both read the old total
    ///    and both spent it. `BEGIN IMMEDIATE` takes the write lock
    ///    before the read, which also holds across processes — the
    ///    portal is per-user, but `--grants-db` can be pointed at a
    ///    shared file and a second portal must not be able to race.
    pub fn try_spend_tokens(
        &self,
        app_id: &str,
        day: &str,
        tokens: i64,
        cap: i64,
    ) -> Result<bool, GrantError> {
        let mut conn = self.conn.lock().expect("grant store lock");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let used: i64 = tx
            .query_row(
                "SELECT tokens FROM quota_usage WHERE app_id = ?1 AND day = ?2",
                rusqlite::params![app_id, day],
                |r| r.get(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(0),
                other => Err(other),
            })?;
        if used.saturating_add(tokens) > cap {
            // Nothing written: a refused request costs the app nothing,
            // so a rejected oversized call cannot be used to burn a
            // neighbour's budget either.
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO quota_usage (app_id, day, tokens) VALUES (?1, ?2, ?3)
             ON CONFLICT(app_id, day) DO UPDATE SET tokens = tokens + ?3",
            rusqlite::params![app_id, day, tokens],
        )?;
        tx.commit()?;
        Ok(true)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
        assert!(s.try_spend_tokens("app.a", "day-1", 100, 1000).unwrap());
        assert!(s.try_spend_tokens("app.a", "day-1", 50, 1000).unwrap());
        assert!(s.try_spend_tokens("app.a", "day-2", 7, 1000).unwrap());
        assert_eq!(s.tokens_used("app.a", "day-1").unwrap(), 150);
        assert_eq!(s.tokens_used("app.a", "day-2").unwrap(), 7);
        assert_eq!(s.tokens_used("app.b", "day-1").unwrap(), 0);
    }

    /// Issue #114. The demonstrated exploit was one oversized request
    /// against a tiny budget: admitted whole, counter left far past the
    /// cap. The request has to fit, and a refused one must leave the
    /// counter untouched.
    #[test]
    fn one_oversized_request_cannot_overshoot_the_budget() {
        let s = store();
        assert!(!s.try_spend_tokens("hog", "day-1", 1000, 5).unwrap());
        assert_eq!(
            s.tokens_used("hog", "day-1").unwrap(),
            0,
            "a refused request must not be charged"
        );
        // And the budget is still fully available to a request that fits.
        assert!(s.try_spend_tokens("hog", "day-1", 5, 5).unwrap());
        assert!(!s.try_spend_tokens("hog", "day-1", 1, 5).unwrap());
        assert_eq!(s.tokens_used("hog", "day-1").unwrap(), 5);
    }

    /// The budget is spent exactly, not approximately: a request landing
    /// on the cap is admitted, one token more is not.
    #[test]
    fn the_cap_is_the_cap() {
        let s = store();
        assert!(s.try_spend_tokens("a", "day-1", 99, 100).unwrap());
        assert!(!s.try_spend_tokens("a", "day-1", 2, 100).unwrap());
        assert!(s.try_spend_tokens("a", "day-1", 1, 100).unwrap());
        assert_eq!(s.tokens_used("a", "day-1").unwrap(), 100);
    }

    /// Concurrency, the third half of #114: the check and the add were
    /// separate lock acquisitions, so two callers both read the old
    /// total. Threads hammer one budget; the sum of what was admitted
    /// must never exceed it.
    #[test]
    fn concurrent_spenders_cannot_race_past_the_cap() {
        let s = Arc::new(store());
        let cap = 100;
        let mut handles = Vec::new();
        for _ in 0..8 {
            let s = Arc::clone(&s);
            handles.push(std::thread::spawn(move || {
                let mut admitted = 0;
                for _ in 0..25 {
                    if s.try_spend_tokens("racer", "day-1", 10, cap).unwrap() {
                        admitted += 10;
                    }
                }
                admitted
            }));
        }
        let admitted: i64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(admitted, cap, "admitted more than the budget allows");
        assert_eq!(s.tokens_used("racer", "day-1").unwrap(), cap);
    }

    /// Issue #113: a "no, not now" is recorded so the portal can see an
    /// app asking repeatedly, without becoming a remembered refusal.
    #[test]
    fn transient_refusals_are_counted_but_do_not_decide_state() {
        let s = store();
        for i in 0..3 {
            s.record_at("nag", "inference", GrantAction::DenyOnce, 1_000 + i)
                .unwrap();
        }
        assert_eq!(s.effective("nag", "inference").unwrap(), Effective::Unset);
        assert_eq!(s.refusals_since("nag", "inference", 0).unwrap(), 3);
        // Only within the window.
        assert_eq!(s.refusals_since("nag", "inference", 1_002).unwrap(), 1);
        // And only for that pair.
        assert_eq!(s.refusals_since("nag", "context", 0).unwrap(), 0);
        assert_eq!(s.refusals_since("other", "inference", 0).unwrap(), 0);
    }

    /// Once the user makes a real decision, earlier hesitation stops
    /// counting — otherwise an app the user allowed would still be
    /// serving a cooldown from before.
    #[test]
    fn a_persistent_decision_clears_the_refusal_count() {
        let s = store();
        for i in 0..5 {
            s.record_at("app", "inference", GrantAction::DenyOnce, 1_000 + i)
                .unwrap();
        }
        assert_eq!(s.refusals_since("app", "inference", 0).unwrap(), 5);
        s.record_at("app", "inference", GrantAction::Allow, 2_000)
            .unwrap();
        assert_eq!(s.refusals_since("app", "inference", 0).unwrap(), 0);
    }

    /// The in-memory store used to have its own schema without the
    /// append-only triggers, so the entire test suite ran against a
    /// store that allowed what the real one forbids.
    #[test]
    fn the_in_memory_store_is_append_only_too() {
        let s = store();
        s.record("app.a", "inference", GrantAction::Allow).unwrap();
        let conn = s.conn.lock().unwrap();
        assert!(
            conn.execute("UPDATE grant_actions SET action='deny'", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM grant_actions", []).is_err());
    }
}
