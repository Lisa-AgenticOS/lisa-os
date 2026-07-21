//! lisa-contextd — the context fabric (`docs/PLAN.md` §5.3).
//!
//! M3 core, first pass: per-user SQLite store (FTS5 lexical index +
//! per-app memory namespaces), file ingestion with provenance tags, and
//! scoped retrieval. Every retrieval is ledgered. This crate is the
//! library; the daemon (D-Bus/portal surface, watchers, embedding
//! pipeline via inferenced, sqlite-vec hybrid ranking) builds on it.
//!
//! Design invariants already enforced here:
//! - chunks carry provenance (`file`, later `mail`/`screen`/`web`) —
//!   provenance is load-bearing (PLAN §5.10);
//! - per-app memory is namespace-isolated at the API (§5.3: an app
//!   never reads another's namespace);
//! - the store is one user-openable SQLite file.

pub mod index;
pub mod memory;
pub mod store;

pub use store::{ContextStore, StoreError};
