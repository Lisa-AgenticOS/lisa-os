# Architecture Decision Records

Any deviation from `docs/PLAN.md` — a dead library, a changed API, a
superseded model, a better idea — gets an ADR *before* the code changes.
Never silently improvise (PLAN §0.4).

## Process

1. Copy the template below to `NNNN-short-slug.md` (next free number).
2. Status flows: `proposed` → `accepted` → (`superseded by NNNN`).
3. Reference the ADR from commits and the affected component README.

## Template

```markdown
# ADR-NNNN: Title

- **Status:** proposed | accepted | superseded by NNNN
- **Date:** YYYY-MM-DD

## Context
What forced a decision.

## Decision
What we chose, stated imperatively.

## Consequences
What gets easier, what gets harder, what we gave up.
```

## Index

- [ADR-0001](0001-arch-immutable-base.md) — Fork Arch; immutable mkosi/UKI/A-B image
- [ADR-0002](0002-rust-zbus-axum.md) — Rust + zbus + axum for daemons
- [ADR-0003](0003-two-track-delivery.md) — Two-track delivery: Lisa Layer, then image
- [ADR-0004](0004-flutter-lane-forge.md) — Flutter app lane + the Forge
- [ADR-0005](0005-gpl2-license.md) — License: GPL-2.0-only, same as the kernel
- [ADR-0006](0006-monorepo-staged-extraction.md) — Monorepo with staged extraction (split triggers, not dates)
- [ADR-0007](0007-fcitx5-addon-cxx.md) — fcitx5-lisa is a thin C++ addon; logic stays daemon-side
- [ADR-0008](0008-portal-standalone-service.md) — Portal is a standalone session service; consent stays in the shell
