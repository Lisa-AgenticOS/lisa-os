//! harness-core — the assistant pillars, ported from the design of
//! [flakerimi/harness](https://github.com/flakerimi/harness) onto Lisa's
//! substrate (ADR-0013, phase 2). The Go harness is the *template*; the
//! engine is Lisa's, so this crate is a plain sync library: no HTTP, no
//! D-Bus, no daemons. The caller sends a [`Turn`]'s request body to an
//! OpenAI-compatible endpoint (ureq, sync — as `cli/lisa` does) and routes
//! any actions through the Agent Bus, where tiers, provenance, undo, and
//! the Ledger apply.
//!
//! The pillars:
//!
//! - [`Session`] — persistent multi-turn conversations, stored on the
//!   context fabric: one JSON value per session in the caller's
//!   `dev.lisaos.Context1` app-memory namespace, behind the [`KvStore`]
//!   seam ([`MemKv`] in tests). [`SessionStore`] does
//!   create/list/load/append/prune; the turn wire shape matches what
//!   the Assistant already persists, so it can adopt multi-conversation
//!   support without new daemon surface.
//! - [`Memory`] — per-scope durable notes (the "second brain"):
//!   [`Memory::remember`] / [`Memory::recall`] (FTS5, with a LIKE
//!   fallback) and [`Memory::digest`], the bounded string a caller
//!   injects into the system prompt each turn.
//! - [`Skill`] — SKILL.md workflow files with progressive disclosure:
//!   the [`Skill::catalog_line`] index goes into every prompt;
//!   [`Skill::body`] is read lazily, only when the workflow is used;
//!   [`LoadReport::resolve`] routes a prompt to a skill with the stack's
//!   deterministic token scoring, and an optional `tools:` allowlist
//!   scopes what a skill may drive.
//! - [`Turn`] — pure composition of one assistant turn: persona + memory
//!   digest + skill catalog + windowed history + user input → an OpenAI
//!   chat-completions request body. No IO.
//!
//! A full turn, composed by a caller:
//!
//! ```
//! # fn main() -> Result<(), harness_core::Error> {
//! # let dir = tempfile::tempdir().unwrap();
//! use harness_core::{MemKv, Memory, Role, SessionStore, Turn};
//!
//! let memory = Memory::open(dir.path().join("memory.db"))?;
//! memory.remember("user", "prefers dark theme", &["ui"])?;
//!
//! // On Lisa the store bridges Context1 app-memory; tests use MemKv.
//! let sessions = SessionStore::new(MemKv::default());
//! let session = sessions.create("demo")?;
//! let session = sessions.append(&session.id, Role::User, "theme this app", None)?;
//!
//! let turn = Turn::new("You are Lisa, an on-device assistant.", "make it dark")
//!     .with_digest(memory.digest("user", 1000)?)
//!     .with_history(session.history(20));
//! let body = turn.request_body(); // → POST to /v1/chat/completions
//! // ... caller sends `body`, reads choices[0].message.content ...
//! sessions.append(&session.id, Role::Assistant, "done — dark theme on", Some("local"))?;
//! # Ok(())
//! # }
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

pub mod memory;
pub mod policy;
pub mod session;
pub mod skill;
pub mod store;
pub mod turn;

pub use memory::{Memory, Note};
pub use session::{Message, Role, Session, SessionInfo, SessionStore, SessionTurn};
pub use skill::{LoadReport, Skill, Skipped};
pub use store::{KvStore, MemKv};
pub use turn::Turn;

/// The one error type for the crate's IO (SQLite, filesystem, KV store).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A [`KvStore`] backend failure (e.g. the Context1 bridge).
    #[error("store error: {0}")]
    Store(String),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    /// A stored value that should parse didn't — surfaced, not
    /// silently dropped, because it means data loss.
    #[error("corrupt stored value: {0}")]
    Corrupt(String),
}

pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
