# ADR-0007: fcitx5-lisa is a C++ addon (thin), logic stays on the daemon side

- **Status:** accepted
- **Date:** 2026-07-22

## Context

PLAN §5.7.3 layer 2 — Writing Tools with universal coverage — is an
**fcitx5 addon**: input-method protocols reach everything that accepts
text (GTK, Qt, Electron/Chromium, terminals, XWayland), which is the
trick that replaces Apple's private toolkit hooks. CLAUDE.md rule 4
allots TypeScript/GJS to shell surfaces and Rust to daemons; it is
silent on input methods.

fcitx5 addons are loaded as shared libraries against the C++
`Fcitx5Core` API (AddonInstance/AddonFactory, InputContext, the event
loop). There is no supported C ABI for addons and no maintained Rust
binding (fcitx5-rs is abandoned pre-1.0); an out-of-process bridge
(stub addon ↔ Rust daemon) would add a hop and a failure mode to the
hottest text path in the OS for no isolation gain — the addon already
runs inside the user's fcitx5 process.

## Decision

Write `ime/fcitx5-lisa` in **C++ against Fcitx5Core, and keep it
thin**: key handling, surrounding-text/selection capture, commit-string
insertion, and one localhost HTTP call to `lisa-inferenced`'s
OpenAI-compat endpoint (§5.1 — documented in §5.6 as the
zero-dependency integration path). All model behavior — prompts,
guided generation, model routing — stays in the daemon. No new
external dependencies: plain POSIX sockets to 127.0.0.1, no libcurl,
no TLS (loopback only). The same "match the substrate's native
language at the boundary, keep it thin" logic already applies to the
portal ("Rust or C matching upstream portal conventions", §5.5).

## Consequences

- The precedent is bounded: C++ is acceptable only where an upstream
  plugin ABI forces it, and such components must stay protocol-thin.
- Build/test needs fcitx5 headers → Linux-only (Arch container in CI,
  like the other Linux-only lanes); macOS dev hosts get compile checks
  from CI, not locally.
- If fcitx5 ever grows a stable C ABI or a maintained Rust binding,
  migrating this thin layer is cheap by construction.
- Dictation (§5.7.5) lands as another input mode in this same addon;
  the audio pipeline stays daemon-side for the same reason the text
  logic does.
