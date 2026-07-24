//! Sessions — persistent multi-turn conversations (the flakerimi/harness
//! "Sessions" pillar). One SQLite file at a caller-supplied path holds any
//! number of sessions; messages are appended per session and read back as
//! a bounded, chronological window for the next turn. Plain sync API —
//! the caller owns threading; there is no daemon.

use crate::{Error, now_millis};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS sessions (
        id         TEXT PRIMARY KEY,
        title      TEXT NOT NULL,
        created_ts INTEGER NOT NULL,
        updated_ts INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS messages (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id TEXT NOT NULL REFERENCES sessions(id),
        ts         INTEGER NOT NULL,
        role       TEXT NOT NULL,
        content    TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS messages_session ON messages(session_id, id);";

/// The role of a chat message — the OpenAI chat-completions roles a
/// session stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            "tool" => Some(Role::Tool),
            _ => None,
        }
    }
}

/// One stored message — the chat-completions shape (role + content).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Message {
            role,
            content: content.into(),
        }
    }
}

/// A session's listing entry (for pickers, `lisa sessions`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    /// Unix milliseconds.
    pub created_ts: i64,
    pub updated_ts: i64,
}

/// A handle on one conversation in the store at `path`.
pub struct Session {
    conn: Mutex<Connection>,
    id: String,
    title: String,
}

impl Session {
    /// Open (creating if needed) the store at `path` and start a new
    /// session titled `title`.
    pub fn create(path: impl AsRef<Path>, title: &str) -> Result<Self, Error> {
        let conn = open_store(path.as_ref())?;
        let id = new_session_id();
        let now = now_millis();
        conn.execute(
            "INSERT INTO sessions (id, title, created_ts, updated_ts) VALUES (?1, ?2, ?3, ?3)",
            params![id, title, now],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            id,
            title: title.to_string(),
        })
    }

    /// Re-open the store at `path` and continue an existing session.
    pub fn resume(path: impl AsRef<Path>, id: &str) -> Result<Self, Error> {
        let conn = open_store(path.as_ref())?;
        let title: Option<String> = conn
            .query_row("SELECT title FROM sessions WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(title) = title else {
            return Err(Error::SessionNotFound(id.to_string()));
        };
        Ok(Self {
            conn: Mutex::new(conn),
            id: id.to_string(),
            title,
        })
    }

    /// Every session in the store, most recently active first.
    pub fn list(path: impl AsRef<Path>) -> Result<Vec<SessionInfo>, Error> {
        let conn = open_store(path.as_ref())?;
        let mut stmt = conn.prepare(
            "SELECT id, title, created_ts, updated_ts FROM sessions ORDER BY updated_ts DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SessionInfo {
                id: r.get(0)?,
                title: r.get(1)?,
                created_ts: r.get(2)?,
                updated_ts: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Append a message; returns its store id. Also bumps the session's
    /// `updated_ts` (this is a conversation log, not the append-only
    /// Ledger — sessions are mutable by design).
    pub fn append(&self, role: Role, content: &str) -> Result<i64, Error> {
        let now = now_millis();
        let mut conn = self.conn.lock().expect("session lock");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO messages (session_id, ts, role, content) VALUES (?1, ?2, ?3, ?4)",
            params![self.id, now, role.as_str(), content],
        )?;
        let msg_id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE sessions SET updated_ts = ?2 WHERE id = ?1",
            params![self.id, now],
        )?;
        tx.commit()?;
        Ok(msg_id)
    }

    /// The most recent `limit` messages, oldest first — the window a
    /// caller feeds into [`crate::Turn::history`].
    pub fn history(&self, limit: usize) -> Result<Vec<Message>, Error> {
        let conn = self.conn.lock().expect("session lock");
        let mut stmt = conn.prepare(
            "SELECT role, content FROM messages
             WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let raw = stmt.query_map(params![self.id, limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut msgs = Vec::new();
        for row in raw {
            let (role, content) = row?;
            let role = Role::parse(&role).ok_or(Error::UnknownRole(role))?;
            msgs.push(Message { role, content });
        }
        msgs.reverse();
        Ok(msgs)
    }
}

fn open_store(path: &Path) -> Result<Connection, Error> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// Unique-enough session id without a uuid/rand dependency: nanosecond
/// timestamp + pid + a process counter, hex-encoded.
fn new_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "s-{nanos:x}-{}-{:x}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_history_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.db");
        let s = Session::create(&path, "demo").unwrap();
        s.append(Role::User, "one").unwrap();
        s.append(Role::Assistant, "two").unwrap();
        s.append(Role::Tool, "three").unwrap();

        let all = s.history(10).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(
            all.iter().map(|m| m.role).collect::<Vec<_>>(),
            vec![Role::User, Role::Assistant, Role::Tool]
        );
        assert_eq!(all[0].content, "one", "chronological within the window");

        // The window is the most recent `limit`, still oldest-first.
        let window = s.history(2).unwrap();
        assert_eq!(
            window,
            vec![
                Message::new(Role::Assistant, "two"),
                Message::new(Role::Tool, "three")
            ]
        );
    }

    #[test]
    fn persist_and_resume_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.db");
        let id;
        {
            let s = Session::create(&path, "notes for friday").unwrap();
            id = s.id().to_string();
            s.append(Role::User, "remember the milk").unwrap();
            s.append(Role::Assistant, "noted").unwrap();
        } // store closed

        let resumed = Session::resume(&path, &id).unwrap();
        assert_eq!(resumed.title(), "notes for friday");
        let history = resumed.history(10).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0], Message::new(Role::User, "remember the milk"));
        resumed.append(Role::User, "and eggs").unwrap();
        assert_eq!(resumed.history(10).unwrap().len(), 3);

        // The listing reflects the session and its activity.
        let listed = Session::list(&path).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].title, "notes for friday");
        assert!(listed[0].updated_ts >= listed[0].created_ts);

        // Unknown ids fail cleanly.
        assert!(matches!(
            Session::resume(&path, "s-nope"),
            Err(Error::SessionNotFound(_))
        ));
    }

    #[test]
    fn sessions_are_isolated_within_one_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.db");
        let a = Session::create(&path, "a").unwrap();
        let b = Session::create(&path, "b").unwrap();
        a.append(Role::User, "hello a").unwrap();
        assert_eq!(b.history(10).unwrap().len(), 0);
        assert_eq!(Session::list(&path).unwrap().len(), 2);
    }
}
