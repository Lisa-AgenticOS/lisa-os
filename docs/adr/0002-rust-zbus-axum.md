# ADR-0002: Rust with zbus + axum for system daemons

- **Status:** accepted — every daemon that serves a bus is Rust on zbus, with
  axum where there is HTTP (`inferenced`, `remoted`). Corrected 2026-08-06:
  the word was "every", and `daemons/modeld` has never been on zbus — it
  declares neither zbus nor axum and ships no unit, because it is the
  content-addressed store `lisa models` links rather than a running service.
  CLAUDE.md rule 5 already carried the same correction.
- **Date:** 2026-07-20
- **Claims:**
  - `symbol:^zbus@daemons/inferenced/Cargo.toml` — zbus where there is a bus
  - `symbol:^axum@daemons/inferenced/Cargo.toml` — axum where there is HTTP
  - `nomatch:zbus@daemons/modeld/Cargo.toml` — and modeld still is not a bus daemon, which is the 2026-08-06 correction

## Context

Four long-running daemons (`inferenced`, `modeld`, `contextd`, `agentd`)
need D-Bus surfaces, an OpenAI-compatible HTTP endpoint, child-process
supervision, and aggressive sandboxing (PLAN §5.1–§5.4). Omarchy's
most-criticized trait is bash as OS substrate (PLAN Appendix E) — we
need testable, memory-safe plumbing from day one.

## Decision

All daemons and the SDK core are Rust. D-Bus via `zbus` (pure Rust, no
libdbus linkage), HTTP via `axum` on tokio, hashing via `blake3`,
storage via SQLite (`contextd`, M3). Engines (llama.cpp et al.) stay
supervised child processes — never linked in-process, so an engine crash
is a restart, not a system-AI outage. Bindings flow Rust core → C ABI →
GObject Introspection (PLAN §5.6).

## Consequences

- One language across daemons keeps the review surface and CI simple;
  clippy `-D warnings` and rustfmt are merge gates.
- zbus/axum are boring, maintained choices; the novelty budget stays on
  the product surface (PLAN §0.5).
- The workspace must stay cross-platform (macOS dev hosts): systemd and
  portal integration are Linux-only crates/units, exercised in CI.
- Python remains for build tooling and evals only; GJS/TS for Shell.
