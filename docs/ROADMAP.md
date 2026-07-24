# Lisa OS — Roadmap & State of the Build

Where the whole project stands against `docs/PLAN.md`, and the path from
here to a real, shippable Lisa. Companion to `docs/STATUS.md` (component
detail) and `docs/PLAN.md` (scope, source of truth).
**Updated: 2026-07-23 (day 2 — Ambient voice loop, LisaCode, SDK tasks; VISION.md added).**

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
| **M3** Context fabric | 🟠 core+hybrid+ACL | FTS5 + provenance + per-app memory ✅; embedding pipeline + hybrid (BM25+cosine) retrieval ✅; scoped-ACL retrieval + fuzz (0 cross-scope leaks) ✅; watchers/live sources left |
| **M4** Surfaces | 🟠 landed, needs polish | overlay + launcher + Ledger app + fcitx5 running on GNOME/hardware; writing-tools/voice/budgets left |
| **M5** Agent Bus | 🟠 landed | lisa-agentd on main: MCP manifests, registry, tier enforcement at the bus, undo journal, injection gate ✅; MCP wire transport + `lisa tools/call/undo` verbs left |
| **§5.11** Remote providers | 🟠 landed | lisa-remoted broker ✅ (openai/anthropic/hf/tinker/together/fireworks + custom), routing ✅, `lisa remote` CLI ✅, hardware-aware fit ✅; image packaging + socket bridge left (Linux-verify) |
| **§5.7.5** Voice / Ambient | 🟠 loop works | STT (whisper) + wake-word ("Hey Lisa") + answer + TTS verified end-to-end (`lisa ambient once`); live-mic capture + on-image packaging left (ADR-0011) |
| **M6** Apps + Forge | 🟠 loop works | **LisaCode** (`lisa forge`) drives the model→jailed-edit→analyze loop end to end (§5.12.1); lisa_ui + lisa_flutter seeds; GUI Forge + app suite + hot-reload left |
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
`dev.lisaos.Inference1` D-Bus with fd-passed streams, embeddings,
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

**Surfaces (M4).** Assistant overlay (`dev.lisaos.Overlay1` + GNOME
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

### Done this session (2026-07-23, day 2)
- 🟠 **Settings › Intelligence (native GNOME panel)**: decided (ADR-0012)
  to ship our own `gnome-control-center` (pinned 50.3) with a native
  **Intelligence** panel in the sidebar — g-c-c has no plugin API, so a
  thin maintained patch (panel dir + two anchored edits) is the only way
  in. Scaffolded `os/packages/gnome-control-center-lisa`: the C panel
  (`CcPanel`/libadwaita, local models via `lisa models catalog --json` +
  a providers bridge), PKGBUILD, and integration — written against the
  fetched 50.3 API; the sed integration is verified against real upstream
  files. **Builds in CI/container, not yet compiled** (macOS host can't
  build g-c-c); not yet wired into the image.
- ✅ **AI panel (standalone, GJS)**: `app.lisaos.Settings` two sections
  (Local models + Providers with per-provider model routing); the panel
  the native g-c-c one bridges to. `lisa models catalog/list --json`.
- ✅ **Scoped-ACL retrieval** (M3 §5.3 acceptance): `contextd::acl` maps a
  granted portal scope to allowed provenance and filters *at the query*, so
  a disallowed chunk can't leak through ranking — a `documents`-scoped
  search never returns a `mail`/`screen` chunk even when it ranks best.
  Deny-by-default on empty/unknown scopes. ACL-leak + fuzz tests (0
  cross-scope leaks) + `lisa context search --scope <scope>` (ledgered as
  `context.search.scoped`).
- ✅ **Vision + Ambient**: `docs/VISION.md` (the "Her, but yours" north
  star) + ADR-0011 (always-on, wake-word-free-*capable*, on-device,
  ledgered — "Hey Lisa" is the confirmed default).
- ✅ **Voice loop, live**: `lisa transcribe` (whisper.cpp), `lisa say`,
  `lisa ambient once` — verified end to end on real audio + Gemma:
  "Hey Lisa, capital of France?" → Paris; a pizza aside → stays quiet.
  whisper-base-en pinned. (Honest finding: the addressed-intent
  classifier over-triggers on a 1B model → wake word is the right
  default; Phase-2 needs a bigger model + a false-accept eval gate.)
- ✅ **LisaCode** (`lisa forge`): the Forge loop runs end to end against a
  live model, tool jail proven (rejects bad paths, feeds them back);
  quality is model-bound (§5.12.1 coder-model / BYO-agent tiering).
- ✅ **SDK**: `liblisa::tasks` (extract/classify/summarize) + the
  recipe-extractor sample (§5.6 acceptance: <40 lines, stock OpenAI
  client, verified live). Model aliases (`lisa`/`default`) so callers
  needn't know the exact id.

### Deferred (needs Linux verification, not done blind)
- Package `lisa-remoted` + Settings into the image + the cross-daemon
  socket bridge (design in `daemons/remoted/README.md`).
- Build whisper.cpp + piper from source into the image (not in Arch
  repos) so voice works on the device.
- The Spotlight-style right-⌘ overlay summon (GNOME keybinding).
- `lisa tools/call/undo` verbs over `dev.lisaos.Agent1` (async D-Bus
  client in the CLI).

## What's left — the path to a real Lisa

**Near term (M2–M3 completion):**
- Agent Bus follow-ups (agentd landed): MCP wire transport, `libs/mcp-bus`,
  `lisa tools/call/undo` verbs, first first-party app that exposes tools.
  (Injection corpus now 600 attempts — 40-technique payload bank × 5
  vectors × 3 targets — clearing the §5.10 500+ bar with 0 unconfirmed
  privileged calls; model-in-the-loop layer still deferred, ADR-0009.)
- Context fabric: sqlite-vec at scale, file/mail/calendar *live* sources,
  watchers, Settings › Intelligence panel. (Embedding pipeline + hybrid
  ranking done; scoped-ACL retrieval + ACL fuzz — 0 cross-scope leaks —
  done: `search_scoped` maps portal scope → allowed provenance and
  filters at the query, `lisa context search --scope documents`.)
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
