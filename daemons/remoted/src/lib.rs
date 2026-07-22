//! lisa-remoted — the BYO remote-provider egress broker (ADR-0008,
//! `docs/PLAN.md` §5.11). The only component besides lisa-modeld with
//! network access; inferenced reaches it over a local unix socket and
//! itself gains no network (CLAUDE.md rule 5). Every remote request is
//! ledgered with the `remote.` "leaves your hardware" marking before
//! egress, and per-scope offload consent defaults to: nothing leaves.

pub mod api;
pub mod consent;
pub mod dbus;
pub mod oauth;
pub mod provision;
pub mod proxy;
pub mod registry;
pub mod secrets;
pub mod service;
