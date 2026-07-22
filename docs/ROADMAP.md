# Lisa OS — Roadmap & State of the Build

Where the whole project stands against `docs/PLAN.md`, and the path from
here to a real, shippable Lisa. Companion to `docs/STATUS.md` (component
detail) and `docs/PLAN.md` (scope, source of truth).
**Updated: 2026-07-23 (overnight full-cycle session — both open PRs landed, providers + hybrid retrieval + Gemma).**

## The one-paragraph state

In four days Lisa went from a planning document to **a bootable,
self-updating, immutable OS that runs a GNOME desktop with AI shell
surfaces on real 2017 iMac hardware**, backed by a working inference
substrate (local models + guided generation + a hardened no-egress
daemon), an append-only audit Ledger enforced as a hard gate, a context
fabric, a portal trust boundary, and — as of tonight — BYO remote model
providers (OpenAI, Anthropic, HuggingFace, Tinker, …). Everything on
`main` is CI-green; releases publish to GitHub and devices self-update
with automatic rollback.

## Milestone scorecard

| Milestone | Status | One-line |
|---|---|---|
| **M0** Bootstrap | done (polish left) | image builds → boots → A/B update + rollback + sysupdate; release channel live |
| **M1** Inference core | ✅ done (LoRA + perf left) | real inference, crash recovery, guided-gen 1000/1000, scheduler, D-Bus, embeddings, multi-model residency |
| **M2** Trust boundary + SDK | 🟠 mostly | portal ✅, Ledger ✅; liblisa guided-gen ✅, sessions/tasks bindings partial |
| **M3** Context fabric | 🟠 core+hybrid | FTS5 + provenance + per-app memory ✅; embedding pipeline + hybrid (BM25+cosine) retrieval ✅; watchers/sources/scoped-ACL left |
| **M4** Surfaces | 🟠 landed, needs polish | overlay + launcher + Ledger app + fcitx5 running on GNOME/hardware; writing-tools/voice/budgets left |
| **M5** Agent Bus | 🟠 landed | lisa-agentd on main: MCP manifests, registry, tier enforcement at the bus, undo journal, injection gate ✅; MCP wire transport + `lisa tools/call/undo` verbs left |
| **§5.11** Remote providers | 🟠 landed | lisa-remoted broker ✅ (openai/anthropic/hf/tinker/together/fireworks + custom), routing ✅, `lisa remote` CLI ✅, hardware-aware fit ✅; image packaging + socket bridge left (Linux-verify) |
| **M6** Apps + Forge | 🟡 seeds | forge-harness + lisa_ui + lisa_flutter skeletons; app suite + Forge app left |
| **M7** Personal node + installer | 🟡 groundwork | remote broker = PCN groundwork; `lisa install` proto-installer; OOBE + WireGuard pairing left |
| **M8** Public alpha ISO | 🟡 channel exists | releases publish; docs site + eval dashboard + security review left |

Legend: ✅ done · 🟠 core done, more to do · 🟡 started/seeded · 🟥 not begun.

## What's actually done (verified, CI-green)

**The OS (M0, Track I + L).** mkosi Arch image: builds, boots on QEMU
*and* a real iMac18,2, A/B slots with boot-counting rollback, real
`systemd-sysupdate` into the inactive slot. Track L layer installs/
uninstalls on vanilla Arch. GitHub Releases *are* the update channel;
`lisa update`/`lisa install`; weekly boot-gated rebuilds carry Arch
security fixes; devices auto-stage via the sysupdate timer. GNOME
desktop lane (gdm, shell, portals, Mesa, NetworkManager) boots to a
usable session on hardware.

**Inference (M1).** `lisa-inferenced`: supervised llama-server children,
real streaming tokens, kill-9 recovery ~2 s, guided generation
(JSON-Schema→GBNF, 1000/1000 sampled gate), QoS preemption <250 ms,
`org.lisa.Inference1` D-Bus with fd-passed streams, embeddings,
multi-model residency with LRU eviction, verified zero egress.
`lisa-modeld`: blake3 content store, hardware profiler (§8 tiers),
resumable pulls, **hardware-aware model fit** (`lisa models catalog`).

**Trust & legibility (M2).** The Ledger: append-only SQLite (triggers
block UPDATE/DELETE), enforced so no inference happens without a ledger
entry first. `xdg-desktop-portal-lisa`: per-app identity, consent,
grants, quotas. Ledger app in the shell.

**Context (M3 core).** `lisa-contextd`: FTS5 file index with provenance
tags + incremental reindex, namespace-isolated per-app memory with
zero-residual wipe.

**Surfaces (M4).** Assistant overlay (`org.lisa.Overlay1` + GNOME
extension), semantic launcher, Ledger app, fcitx5 writing-tools addon —
all with logic unit-tested; live on the iMac desktop.

**Remote providers (§5.11, tonight).** `lisa-remoted` egress broker:
OpenAI/Anthropic/Tinker/Together/Fireworks/**HuggingFace** + custom
URLs, encrypted per-key creds, per-scope consent defaulting OFF, every
egress ledgered in the "leaves your hardware" marking.

## Done this overnight session (2026-07-22 → 07-23)

- ✅ **Both open PRs landed**: M5 Agent Bus (#5) and §5.11 remote
  providers (#6), rebased onto main, ADRs 0009/0010 reconciled.
- ✅ **HuggingFace** provider (verified router) + openai/anthropic/
  tinker/together/fireworks; `lisa remote` CLI (list/add/key/consent),
  live-tested against the real broker.
- ✅ **Remote routing**: `lisa ask --model remote:<provider>:<model>`
  proxies to the broker over its unix socket (round-trip tested).
- ✅ **Hardware-aware** `lisa models catalog` (runs-here vs offload).
- ✅ **Gemma 3 1B** pinned in the catalog (verified load+generate) +
  `lisa models get <id>` — the iMac gets local inference in one command.
- ✅ **M3 hybrid retrieval**: embedding pipeline + BM25+cosine ranking
  in contextd; `lisa context index --embed`, `search --hybrid`.
- ✅ **Device fixes** (from field photos): boots to GNOME
  (graphical.target), NetworkManager auto-enabled, Settings app shipped,
  openssh for headless access.

### Deferred (needs Linux verification, not done blind)
- Package `lisa-remoted` + Settings into the image + the cross-daemon
  socket bridge (design in `daemons/remoted/README.md`).
- `lisa tools/call/undo` verbs over `org.lisa.Agent1` (async D-Bus
  client in the CLI).

## What's left — the path to a real Lisa

**Near term (M2–M3 completion):**
- Merge the agentd PR (#5) → Agent Bus: MCP wire transport, `libs/mcp-bus`,
  `lisa tools/call/undo` verbs, full 500-attempt injection corpus.
- Context fabric: embedding pipeline + hybrid ranking (sqlite-vec),
  file/mail/calendar sources, watchers, ACL fuzz suite (0 cross-scope
  leaks), Settings › Intelligence panel.
- liblisa SDK: session/tasks/memory bindings + the <40-line recipe
  sample; the OpenAI-compat zero-dep path is already documented.

**Mid term (M4–M6):**
- Writing-tools layer 1 (GTK module), voice v1 (whisper + wake word),
  screen-context VLM, §5.7 latency budgets measured on hardware.
- First-party apps: Notes + Recorder (Flutter lane), Files/Mail/Photos
  patches. The Forge app on `forge-harness`.
- LoRA adapter lane (Tinker credits available) for task specialization.

**Longer term (M7–M8):**
- Guided first-boot OOBE (real users, disk/TPM-LUKS, tier-aware model
  download) replacing the provisional lisa/lisa login.
- Personal Compute Node: WireGuard pairing to your own bigger box as a
  `remote:personal` tier.
- Nonfree NVIDIA/CUDA image variant.
- Public alpha ISO: docs site, SDK quickstarts, eval dashboard,
  security-review pass.

**Hardening backlog (cross-cutting, before anyone but us runs it):**
- Signed sysupdate manifests (drop `Verify=no`); dm-verity on root;
  `/etc` overlay with factory reset; Arch base snapshot-pinned in
  release builds (`os/repo-tools/snapshot.sh` exists, not wired);
  SSH key-auth + hardening (password `lisa/lisa` is provisional);
  Broadcom Wi-Fi firmware confirmed per field device.

## The dev ⇄ field loop (how we ship)

1. Code + merge on the dev machine; CI gates every push (pre-push hook
   runs fmt+clippy locally too).
2. `gh workflow run release` (or the weekly cron) builds a boot-gated
   image and publishes a GitHub Release.
3. Field iMac: `lisa update --reboot` (or the daily sysupdate timer);
   boot-counting auto-rolls-back a bad release.
4. Real-hardware findings (four bugs so far: root discovery, initrd
   keyboard, NetworkManager preset, missing Settings) feed back as
   fixes → next release.
