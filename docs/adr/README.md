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
- [ADR-0009](0009-agent-bus-core.md) — Agent Bus core: `dev.lisaos.Agent1`, tier enforcement at the bus, staged MCP transport
- [ADR-0010](0010-remote-providers.md) — BYO remote providers via the `lisa-remoted` egress broker
- [ADR-0011](0011-ambient-assistant.md) — Lisa Ambient: always-on, wake-word-free, on-device, ledgered
- [ADR-0012](0012-gnome-control-center-lisa-panel.md) — Native "Intelligence" panel in a forked gnome-control-center
- [ADR-0013](0013-harness-intents-and-coding-agent.md) — The Lisa harness: intents + a coding agent on the existing substrate
- [ADR-0014](0014-lisa-ui-material-fork.md) — lisa_ui is the kit Lisa apps import — Material-backed now, vendored fork later
- [ADR-0015](0015-assistant-app.md) — Persistent Assistant chat window
- [ADR-0016](0016-reverse-dns-naming.md) — Reverse-DNS identifiers move to the real domains (dev.lisaos.* / app.lisaos.*)
- [ADR-0017](0017-plymouth-in-initrd.md) — Plymouth + the lisa theme move into the mkosi-initrd
- [ADR-0018](0018-var-pinned-partuuid.md) — /var is mounted by partition LABEL, not by UUID
- [ADR-0019](0019-dedicated-home-partition.md) — Dedicated /home partition on fresh installs, weight-split with var
- [ADR-0020](0020-app-update-channel.md) — App updates decoupled from the OS image
- [ADR-0021](0021-aarch64-lane.md) — aarch64 image lane on an Arch Linux ARM base
