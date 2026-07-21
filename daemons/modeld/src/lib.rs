//! lisa-modeld — model catalog & content-addressed store.
//!
//! Spec: `docs/PLAN.md` §5.2. This crate is both the daemon (see `main.rs`)
//! and the library the `lisa` CLI uses for local store operations until the
//! D-Bus surface lands in M1.

pub mod catalog;
pub mod fetch;
pub mod profile;
pub mod store;

pub use store::{ModelStore, RefEntry, StoreError, VerifyReport};
