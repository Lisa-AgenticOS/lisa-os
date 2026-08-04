# ADR-0002: Rust with zbus + axum for system daemons

- **Status:** accepted — every daemon under `daemons/` is Rust on zbus, with
  axum where there is HTTP.
- **Date:** 2026-07-20

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
