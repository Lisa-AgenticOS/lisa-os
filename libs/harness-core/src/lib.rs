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
//! - [`Session`] — persistent multi-turn conversations: SQLite-backed,
//!   append user/assistant/tool messages, windowed [`Session::history`],
//!   [`Session::resume`] across runs.
//! - [`Memory`] — per-scope durable notes (the "second brain"):
//!   [`Memory::remember`] / [`Memory::recall`] (FTS5, with a LIKE
//!   fallback) and [`Memory::digest`], the bounded string a caller
//!   injects into the system prompt each turn.
//! - [`Skill`] — SKILL.md workflow files with progressive disclosure:
//!   the [`Skill::catalog_line`] index goes into every prompt;
//!   [`Skill::body`] is read lazily, only when the workflow is used.
//! - [`Turn`] — pure composition of one assistant turn: persona + memory
//!   digest + skill catalog + windowed history + user input → an OpenAI
//!   chat-completions request body. No IO.
//!
//! A full turn, composed by a caller:
//!
//! ```
//! # fn main() -> Result<(), harness_core::Error> {
//! # let dir = tempfile::tempdir().unwrap();
//! let memory = harness_core::Memory::open(dir.path().join("memory.db"))?;
//! memory.remember("user", "prefers dark theme", &["ui"])?;
//!
//! let session = harness_core::Session::create(dir.path().join("chat.db"), "demo")?;
//! session.append(harness_core::Role::User, "theme this app")?;
//!
//! let turn = harness_core::Turn::new("You are Lisa, an on-device assistant.", "make it dark")
//!     .with_digest(memory.digest("user", 1000)?)
//!     .with_history(session.history(20)?);
//! let body = turn.request_body(); // → POST to /v1/chat/completions
//! // ... caller sends `body`, reads choices[0].message.content ...
//! session.append(harness_core::Role::Assistant, "done — dark theme on")?;
//! # Ok(())
//! # }
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

pub mod memory;
pub mod session;
pub mod skill;
pub mod turn;

pub use memory::{Memory, Note};
pub use session::{Message, Role, Session, SessionInfo};
pub use skill::{LoadReport, Skill, Skipped};
pub use turn::Turn;

/// The one error type for the crate's IO (SQLite + filesystem).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("stored message has unknown role `{0}`")]
    UnknownRole(String),
}

pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
