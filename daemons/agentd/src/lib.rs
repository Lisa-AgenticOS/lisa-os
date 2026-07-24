//! lisa-agentd — the Agent Bus (`docs/PLAN.md` §5.4, M5 first slice).
//!
//! Apps are MCP servers (manifest schema in PLAN Appendix B); this
//! daemon is the system MCP client/host: it keeps the registry of
//! installed servers, mediates discovery, and executes calls under
//! **bus-enforced** confirmation tiers — read → silent, write → chip,
//! destructive → modal — with rule-6 provenance escalation (any
//! untrusted provenance in the trigger chain escalates one tier; PLAN
//! §5.10, Appendix C). Every call is ledgered before it happens (no
//! ledger entry, no action), and every executed privileged call records
//! its app-declared compensation in the undo journal (`lisa undo`).
//!
//! First slice (this crate, host-independent — builds and tests on
//! macOS and Linux):
//! - `manifest`  — Appendix B manifest parsing + validation, plus a
//!   minimal structural args validator;
//! - `registry`  — installed-manifest registry + tool discovery;
//! - `tier`      — the confirmation-tier policy (fail-closed);
//! - `bus`       — the call state machine (request → confirm/deny →
//!   execute), Ledger attribution, undo journal wiring;
//! - `journal`   — the SQLite undo journal (compensation calls);
//! - `dbus`      — the `dev.lisaos.Agent1` surface (session bus on real
//!   systems; zbus p2p in tests).
//!
//! Deferred to the next M5 slices (ADR-0009): the MCP wire transport
//! (per-app unix socket + activation) behind the `bus::Dispatcher`
//! trait, `libs/mcp-bus` extraction, `lisa tools/call/undo` CLI verbs,
//! btrfs-snapshot compensation for file ops, and the model-in-the-loop
//! injection e2e.
//!
//! Egress rule (CLAUDE.md rule 5): this daemon never gets network
//! access — no network dependency may be added here.

pub mod bus;
pub mod dbus;
pub mod journal;
pub mod manifest;
pub mod registry;
pub mod tier;
