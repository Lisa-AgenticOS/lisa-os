# lisa_flutter — parked: the Dart SDK bindings

Spec: docs/PLAN.md §5.12. Milestone: M6. Governance: **ADR-0047** (GJS +
GTK4/Adwaita is the one toolkit), ADR-0004 (history).

**Status: parked** (ADR-0047 §2). Unshipped, unproven on hardware, not
the default — and, unlike `libs/lisa_ui`, never started: this is a
scaffold placeholder, not an implementation.

The earlier version of this file said *"not started — blocked on the
ADR-0004 spike"*, which read as a queue position. It is not queued.
ADR-0004's Flutter lane was superseded as the default by ADR-0047 on
2026-08-04, and the spike it was blocked on is not going to unblock it.

## What it was going to be

Dart bindings mirroring liblisa — sessions, guided generation, tasks,
memory, tools — over D-Bus via the `dbus` Dart package, with the
OpenAI-compatible endpoint as a fallback transport.

That gap is already covered for every other language: `liblisa` exposes
a C ABI with Rust, Python, JS and Vala bindings plus the
OpenAI-compatible endpoint, which is why ADR-0047 could conclude that
"an app framework we do not ship" was never what stood between a
third-party developer and Lisa.

## To build a Lisa app

Not with this. GJS + GTK4/Adwaita, `docs/ANATOMY-OF-AN-APP.md`,
`skills/build-lisa-app/SKILL.md`, and `lisa dev check` (ADR-0050) as the
authority on whether what you wrote is valid.

## Limits

- Nothing is implemented; there is no API to call.
- Nothing here is built or tested by `just ci`.
- Deleted by nothing: ADR-0047 §2 keeps the lane in the tree so the
  history stays intact and reversing costs four files.
