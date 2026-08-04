# Architecture Decision Records

Any deviation from `docs/PLAN.md` — a dead library, a changed API, a
superseded model, a better idea — gets an ADR *before* the code changes.
Never silently improvise (PLAN §0.4).

## What is actually built

An ADR records a decision, not a delivery. "Accepted" says we chose
something; it says nothing about whether it exists, and 21 of the 49
records below carry no status line at all — so this question could not
be answered by reading them, which is why the table exists.

Read it as: **what would a person find on a device today.**

**The table stops at 0038.** ADR-0039 through ADR-0049 have no row yet;
an entry here is expected to name its evidence (see the note under the
table), and nobody has done that work. Absence from the table means
"unassessed", not "not built".

| ADR | Built? | What is missing, and where it is tracked |
|---|---|---|
| 0001 immutable base | yes | — |
| 0002 rust/zbus/axum | yes | — |
| 0003 two-track delivery | yes | Track I ships; Track L installs onto stock Arch |
| 0004 flutter lane | partial | SDK is fetched on demand, not on the image (#37) |
| 0005 GPL-2.0 | yes | — |
| 0006 monorepo | yes | — |
| 0007 fcitx5 addon | yes | — |
| 0008 portal standalone | yes | Installed since v20260730.55 (#153) |
| 0009 agent bus core | yes | — |
| 0010 remote providers | yes | PKCE state fixed (#110); needs a live sign-in to confirm |
| 0011 ambient assistant | partial | The middle is built and verified: `lisa transcribe` (whisper.cpp) → `ambient classify` → `say` (piper), driven from an audio FILE. Missing both ends — live mic capture, wake word, push-to-talk — and the `voiced` daemon. Nothing is installed on a device: no whisper, no piper, no ASR/TTS model (#158) |
| 0012 control-center panel | yes | — |
| 0013 harness intents | partial | Sessions/Memory/Skills done. Soul partial; Crons, Hands, Background tasks, Self-improvement not started |
| 0014 lisa_ui fork | yes | — |
| 0015 assistant app | partial | Read-tier tools work via Harness1; no write tier, no memory across conversations (#157) |
| 0016 reverse-DNS naming | yes | — |
| 0017 plymouth in initrd | yes | Splash→desktop handoff still gaps (#26) |
| 0018 /var pinned PARTUUID | yes | — |
| 0019 dedicated /home | yes | — |
| 0020 app update channel | yes | — |
| 0021 aarch64 lane | partial | Container-verified; no published image |
| 0022 rescue boot path | partial | Phase 1 (ESP self-repair) done; user-survivable rescue open (#23) |
| 0023 slim core, /var grows | partial | Zen migration incomplete (#89) |
| 0024 CS8409 codec | partial | Packaged; speakers still silent on the reference iMac (#44) |
| 0025 one agent loop | yes | The Assistant runs on `dev.lisaos.Harness1`, which reaches the Agent Bus through `bus-tools` — one loop, as the ADR asks |
| 0026 native DRM in initrd | yes | — |
| 0027 flutter on device | partial | (#37, #48) |
| 0028 initrd overlay | yes | — |
| 0029 hard guardrails | partial | Phases 1–2 done; Landlock confinement open (#53); `lisa suggest` still emits a shell string (#88) |
| 0030 guardrail boundary | yes | The principle now has teeth: #145 and #55 both closed against it |
| 0031 server mode | **no** | No `serverd`, no `lisa serve` |
| 0032 construct/lisa contract | yes | harness-core is the shared level |
| 0033 identity from transport | partial | portal, contextd, remoted, agentd done; sweep for remaining callers unfinished |
| 0034 user-scope dev tooling | **no** | `lisa dev` not built (#130) |
| 0035 desktop is a prompt | partial | §4 consent split done (#145); §2 prompt-in-the-dock not started |
| 0036 assistant acts on its own | **no** | Depends on 0025 |
| 0037 browser is a Lisa app | partial | Surfer ships; write tools and the agent surface open (#146) |
| 0038 Lisa Desktop — hard fork of GNOME Shell | accepted, no code | Forks the Shell's JS, NOT Mutter. Supersedes PLAN §3's "we patch, we don't fork the Shell yet" — this is the phase-3 decision that line deferred |

Two decisions have **no** implementation at all — 0031 and 0036 — and 0011 has a tested pipeline with no way to speak into it.

An earlier version of this table listed 0025 among them, which was
wrong: the Assistant is on the harness and calls Agent Bus tools. The
error came from grepping the window for `Agent1` and finding tooltips,
rather than following `RunSync` to `dev.lisaos.Harness1`. A table of
what is built is worth less than nothing if its entries are inferred
from a grep, so entries here are expected to name the evidence.

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
- [ADR-0022](0022-rescue-boot-path.md) — User-survivable rescue boot path (pinned rescue UKI + self-repair)
- [ADR-0023](0023-slim-core-var-grows.md) — Slim core, /var grows: apps and heavy payloads leave the image
- [ADR-0024](0024-apple-cs8409-out-of-tree-codec.md) — Out-of-tree CS8409 codec module for Apple speakers
- [ADR-0025](0025-one-agent-loop.md) — One agent loop: sessions, memory, skills and every tool family in a single harness
- [ADR-0026](0026-native-drm-in-initrd.md) — The native GPU driver + its firmware ride the initrd
- [ADR-0027](0027-flutter-on-device-aarch64-and-forged-app-launch.md) — The Flutter lane on-device: aarch64 SDK, forged-app build + launch, and where skills live
- [ADR-0028](0028-initrd-overlay-mechanism.md) — Files reach the default initrd through `io.mkosi.initrd`, not `mkosi.initrd/`
- [ADR-0029](0029-hard-guardrails-for-agent-actions.md) — Hard guardrails for agent actions: deterministic policy outside the model
- [ADR-0030](0030-the-guardrail-boundary.md) — The guardrail boundary: probabilistic inside, logical outside — and the owner is outside
- [ADR-0031](0031-server-mode-two-edges-and-artifact-publishing.md) — Server mode, the two network edges, and artifact publishing (proposed)
- [ADR-0032](0032-construct-and-lisa-one-contract-two-levels.md) — Construct and Lisa: one contract, two levels (proposed)
- [ADR-0033](0033-identity-comes-from-the-transport.md) — Identity comes from the transport, not the message (`libs/lisa-peer`)
- [ADR-0034](0034-lisa-dev-user-scope-tooling.md) — `lisa dev`: developer tooling in the user's home, rootless (proposed)
- [ADR-0035](0035-the-desktop-is-a-prompt.md) — The desktop is a prompt: a floating dock-prompt, no top bar (proposed)
- [ADR-0036](0036-an-assistant-that-acts-on-its-own.md) — An assistant that acts on its own: triggers, trust, and what happens when nobody is watching (proposed)
- [ADR-0037](0037-the-browser-is-a-lisa-app.md) — Browser: the web becomes an agent surface, not a vendored binary (Surfer)
- [ADR-0038](0038-lisa-desktop-a-hard-fork-of-gnome-shell.md) — Lisa Desktop: a hard fork of GNOME Shell (not Mutter)
- [ADR-0039](0039-the-split-and-the-package-index.md) — The split, and the `[lisa]` package index that makes it work
- [ADR-0040](0040-docs-live-with-the-code-no-docs-repo.md) — Docs live with the code; lisaos.dev renders them; no docs repo
- [ADR-0041](0041-package-signing-and-the-trust-chain.md) — Package signing and the trust chain (key custody, two-phase SigLevel)
- [ADR-0042](0042-field-device-keyring-policy.md) — The field device runs a blank login keyring (and why that is honest)
- [ADR-0043](0043-the-model-knows-the-os-through-retrieval.md) — The model knows the OS through retrieval, never through the prompt
- [ADR-0044](0044-retrieval-receipts.md) — Retrieval receipts: contextd vouches for what it returned (proposed)
- [ADR-0045](0045-calver-for-the-image-semver-for-the-contracts.md) — CalVer for the image, SemVer for the contracts
- [ADR-0046](0046-capability-before-storefront.md) — Capability before storefront: what must be true before Lisa distributes somebody else's app
- [ADR-0047](0047-one-toolkit-gjs-gtk4.md) — One toolkit: GJS + GTK4/Adwaita is the default, Flutter is parked
- [ADR-0048](0048-lisa-desktop-is-a-desktop-not-a-patched-gnome.md) — Lisa Desktop is a desktop, not a patched GNOME (write the apps; core vs. store)
- [ADR-0049](0049-every-app-is-an-agent-surface.md) — Every app is an agent surface: install is the grant, the tier is the gate, the registry is the authority
