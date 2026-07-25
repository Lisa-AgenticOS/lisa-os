//! Sessions — persistent multi-turn conversations (the flakerimi/harness
//! "Sessions" pillar), stored on Lisa's context fabric instead of a
//! private database: each session is one JSON value in the caller's
//! `dev.lisaos.Context1` app-memory namespace (key `session/<id>`), plus
//! one index value (key `sessions`) listing them. That is the same
//! substrate the Assistant already persists its single conversation
//! through (`shell/assistant/lib/model.js`, key `conversation`), and the
//! turn wire shape matches its `{role, text, model}` payload — so the
//! Assistant can adopt multi-conversation support by pointing at these
//! keys, with no new daemon surface.
//!
//! [`crate::KvStore`] is the seam: a `Context1` D-Bus bridge on Lisa,
//! [`crate::MemKv`] in tests. One writer per app namespace is assumed
//! (the app owns its namespace); the index is read-modify-write.
//!
//! Stored values are never trusted to parse: a corrupt index degrades to
//! empty / junk entries dropped (the Assistant's own convention), while
//! a corrupt session record surfaces as [`Error::Corrupt`] — losing a
//! conversation silently is worse than an error.

use crate::store::KvStore;
use crate::{Error, now_millis};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// The index key: a JSON array of [`SessionInfo`].
pub const INDEX_KEY: &str = "sessions";

/// Per-session key prefix; the full key is `session/<id>`.
pub const SESSION_KEY_PREFIX: &str = "session/";

/// The app-memory key holding session `id`.
pub fn session_key(id: &str) -> String {
    format!("{SESSION_KEY_PREFIX}{id}")
}

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

/// One message in a [`crate::Turn`]'s history window (role + content).
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

/// One logged turn. The wire name for `content` is `text`, and `model`
/// serializes as `null` when absent — byte-compatible with the turn
/// shape the Assistant already stores (`serializeConversation` in
/// `shell/assistant/lib/model.js`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTurn {
    pub role: Role,
    #[serde(rename = "text")]
    pub content: String,
    /// The model that produced an assistant turn; `None` for user turns.
    #[serde(default)]
    pub model: Option<String>,
}

/// A session's listing entry — what the index key stores, for pickers
/// and `lisa sessions`-style listings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    /// Unix milliseconds.
    pub created_ts: i64,
    pub updated_ts: i64,
}

/// One conversation, loaded whole (a session is one KV value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    /// Unix milliseconds.
    pub created_ts: i64,
    pub updated_ts: i64,
    #[serde(default)]
    pub turns: Vec<SessionTurn>,
}

impl Session {
    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            title: self.title.clone(),
            created_ts: self.created_ts,
            updated_ts: self.updated_ts,
        }
    }

    /// The most recent `limit` turns as [`Message`]s, oldest first — the
    /// window a caller feeds into [`crate::Turn::history`].
    pub fn history(&self, limit: usize) -> Vec<Message> {
        let skip = self.turns.len().saturating_sub(limit);
        self.turns[skip..]
            .iter()
            .map(|t| Message::new(t.role, t.content.clone()))
            .collect()
    }
}

/// Session CRUD over one app-memory namespace.
pub struct SessionStore<S: KvStore> {
    kv: S,
}

impl<S: KvStore> SessionStore<S> {
    pub fn new(kv: S) -> Self {
        SessionStore { kv }
    }

    /// The underlying store (e.g. to share it with other pillars).
    pub fn kv(&self) -> &S {
        &self.kv
    }

    /// Start a new session titled `title`: writes its record and adds it
    /// to the index.
    pub fn create(&self, title: &str) -> Result<Session, Error> {
        let now = now_millis();
        let session = Session {
            id: new_session_id(),
            title: title.to_string(),
            created_ts: now,
            updated_ts: now,
            turns: Vec::new(),
        };
        self.write(&session)?;
        let mut index = self.list()?;
        index.insert(0, session.info());
        self.write_index(&index)?;
        Ok(session)
    }

    /// Every session in the namespace, most recently active first. A
    /// missing, tombstoned, or corrupt index reads as empty; junk
    /// entries inside a well-formed array are dropped, not trusted.
    pub fn list(&self) -> Result<Vec<SessionInfo>, Error> {
        let raw = match self.kv.get(INDEX_KEY)? {
            Some(raw) if !raw.is_empty() => raw,
            _ => return Ok(Vec::new()),
        };
        let entries: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
        let mut index: Vec<SessionInfo> = entries
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();
        // Stable sort: ties keep insertion order (newest inserted first).
        index.sort_by_key(|e| std::cmp::Reverse(e.updated_ts));
        Ok(index)
    }

    /// Load session `id` whole. Missing and tombstoned keys are
    /// [`Error::SessionNotFound`]; an unparseable record is
    /// [`Error::Corrupt`].
    pub fn load(&self, id: &str) -> Result<Session, Error> {
        let raw = match self.kv.get(&session_key(id))? {
            Some(raw) if !raw.is_empty() => raw,
            _ => return Err(Error::SessionNotFound(id.to_string())),
        };
        serde_json::from_str(&raw).map_err(|e| Error::Corrupt(format!("session {id}: {e}")))
    }

    /// Append one turn to session `id`, bumping its `updated_ts` in both
    /// the record and the index (re-adding it to the index if it was
    /// somehow lost). Returns the updated session.
    pub fn append(
        &self,
        id: &str,
        role: Role,
        content: &str,
        model: Option<&str>,
    ) -> Result<Session, Error> {
        let mut session = self.load(id)?;
        session.turns.push(SessionTurn {
            role,
            content: content.to_string(),
            model: model.map(str::to_string),
        });
        session.updated_ts = now_millis();
        self.write(&session)?;
        // Move the entry to the front (re-adding it if it was somehow
        // lost): array position breaks same-millisecond ties, so the
        // listing's activity order is exact even within one ms.
        let mut index = self.list()?;
        index.retain(|e| e.id != session.id);
        index.insert(0, session.info());
        self.write_index(&index)?;
        Ok(session)
    }

    /// Keep the `keep` most recently active sessions and remove the
    /// rest (tombstoned on substrates without per-key delete). Returns
    /// the removed ids, oldest last.
    pub fn prune(&self, keep: usize) -> Result<Vec<String>, Error> {
        let index = self.list()?;
        if index.len() <= keep {
            return Ok(Vec::new());
        }
        let (kept, dropped) = index.split_at(keep);
        for info in dropped {
            self.kv.remove(&session_key(&info.id))?;
        }
        self.write_index(kept)?;
        Ok(dropped.iter().map(|i| i.id.clone()).collect())
    }

    fn write(&self, session: &Session) -> Result<(), Error> {
        let json = serde_json::to_string(session).expect("session serializes");
        self.kv.set(&session_key(&session.id), &json)
    }

    fn write_index(&self, index: &[SessionInfo]) -> Result<(), Error> {
        let json = serde_json::to_string(index).expect("index serializes");
        self.kv.set(INDEX_KEY, &json)
    }
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
    use crate::store::MemKv;

    fn store() -> SessionStore<MemKv> {
        SessionStore::new(MemKv::default())
    }

    #[test]
    fn create_append_load_round_trip() {
        let store = store();
        let s = store.create("demo").unwrap();
        assert!(s.turns.is_empty());
        assert_eq!(s.created_ts, s.updated_ts);

        store
            .append(&s.id, Role::User, "theme this app", None)
            .unwrap();
        let after = store
            .append(&s.id, Role::Assistant, "done", Some("qwen3"))
            .unwrap();
        assert_eq!(after.turns.len(), 2);
        assert!(after.updated_ts >= s.updated_ts);

        let loaded = store.load(&s.id).unwrap();
        assert_eq!(loaded, after);
        assert_eq!(loaded.turns[0].model, None);
        assert_eq!(loaded.turns[1].model.as_deref(), Some("qwen3"));

        assert!(matches!(
            store.load("s-nope"),
            Err(Error::SessionNotFound(_))
        ));
    }

    #[test]
    fn wire_shape_matches_the_assistants_conversation_payload() {
        // The Assistant writes these same keys from GJS
        // (shell/assistant/lib/sessions.js, issue #25) and rewrites
        // whole records, so both the turn shape and the field order are
        // contract, not implementation detail.
        let store = store();
        let s = store.create("demo").unwrap();
        store.append(&s.id, Role::User, "hi", None).unwrap();
        store
            .append(&s.id, Role::Assistant, "hello", Some("qwen3"))
            .unwrap();

        let raw = store.kv().get(&session_key(&s.id)).unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            value["turns"],
            serde_json::json!([
                {"role": "user", "text": "hi", "model": null},
                {"role": "assistant", "text": "hello", "model": "qwen3"},
            ])
        );
        let head = format!(
            "{{\"id\":\"{}\",\"title\":\"demo\",\"created_ts\":{},\"updated_ts\":",
            s.id, s.created_ts
        );
        assert!(raw.starts_with(&head), "field order changed: {raw}");
        assert!(raw.ends_with("]}"), "turns stays last: {raw}");
    }

    #[test]
    fn list_orders_by_activity_and_appending_bumps() {
        let store = store();
        let a = store.create("a").unwrap();
        let b = store.create("b").unwrap();
        assert_eq!(
            store
                .list()
                .unwrap()
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            vec![b.id.as_str(), a.id.as_str()],
            "newest first on ties"
        );

        // Sessions are isolated; activity reorders the listing.
        store.append(&a.id, Role::User, "hello a", None).unwrap();
        assert!(store.load(&b.id).unwrap().turns.is_empty());
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, a.id, "appended session is most recent");
        assert_eq!(listed[0].title, "a");
    }

    #[test]
    fn history_windows_oldest_first() {
        let store = store();
        let s = store.create("demo").unwrap();
        store.append(&s.id, Role::User, "one", None).unwrap();
        store.append(&s.id, Role::Assistant, "two", None).unwrap();
        let s = store.append(&s.id, Role::Tool, "three", None).unwrap();

        let all = s.history(10);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0], Message::new(Role::User, "one"));

        let window = s.history(2);
        assert_eq!(
            window,
            vec![
                Message::new(Role::Assistant, "two"),
                Message::new(Role::Tool, "three")
            ]
        );
    }

    #[test]
    fn prune_keeps_the_most_recent_and_removes_the_rest() {
        let store = store();
        let old = store.create("old").unwrap();
        let mid = store.create("mid").unwrap();
        let hot = store.create("hot").unwrap();
        store
            .append(&mid.id, Role::User, "still busy", None)
            .unwrap();

        assert!(store.prune(3).unwrap().is_empty(), "nothing over the cap");
        let removed = store.prune(2).unwrap();
        assert_eq!(removed, vec![old.id.clone()], "least recent goes");
        assert!(matches!(
            store.load(&old.id),
            Err(Error::SessionNotFound(_))
        ));
        assert_eq!(store.list().unwrap().len(), 2);
        assert!(store.load(&mid.id).is_ok());
        assert!(store.load(&hot.id).is_ok());

        // prune(0) clears the namespace's sessions entirely.
        let removed = store.prune(0).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn tombstoned_records_read_as_absent() {
        // A Context1-shaped store can only tombstone (empty string).
        struct GetSetOnly(MemKv);
        impl KvStore for GetSetOnly {
            fn get(&self, key: &str) -> Result<Option<String>, Error> {
                self.0.get(key)
            }
            fn set(&self, key: &str, value: &str) -> Result<(), Error> {
                self.0.set(key, value)
            }
        }
        let store = SessionStore::new(GetSetOnly(MemKv::default()));
        let s = store.create("doomed").unwrap();
        store.prune(0).unwrap();
        assert_eq!(
            store.kv().get(&session_key(&s.id)).unwrap().as_deref(),
            Some(""),
            "tombstone, not deletion"
        );
        assert!(matches!(store.load(&s.id), Err(Error::SessionNotFound(_))));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn stored_junk_degrades_instead_of_breaking_startup() {
        let store = store();
        let s = store.create("demo").unwrap();

        // Corrupt index: unparseable → empty; junk entries → dropped.
        store.kv().set(INDEX_KEY, "not json").unwrap();
        assert!(store.list().unwrap().is_empty());
        store
            .kv()
            .set(
                INDEX_KEY,
                &format!(
                    "[{{\"bogus\": true}}, {}]",
                    serde_json::to_string(&s.info()).unwrap()
                ),
            )
            .unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, s.id);

        // A corrupt session record is an explicit error, not silent loss.
        store.kv().set(&session_key(&s.id), "{broken").unwrap();
        assert!(matches!(store.load(&s.id), Err(Error::Corrupt(_))));

        // Appending to a session missing from the index self-heals it.
        store.write(&s).unwrap();
        store.kv().set(INDEX_KEY, "[]").unwrap();
        store.append(&s.id, Role::User, "hi", None).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
    }
}
