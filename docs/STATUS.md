# Lisa OS — project status & session handoff

Living snapshot of where the build actually is, so any machine (or a
fresh Claude Code session) can pick up without reconstructing context.
`docs/PLAN.md` is still the source of truth for scope; this is the
"where are we on it" companion. **Last updated: 2026-07-24.**

## TL;DR

Three days from planning doc to a **bootable, self-updating OS with a
public release channel**. The inference substrate (M1) is functionally
complete; M2 (Ledger) and M3 (context fabric) have working cores. Every
claim below is enforced by CI on `main`, not aspirational.

- Repo: **github.com/Lisa-AgenticOS/lisa-os** · License: GPL-2.0-only (ADR-0005)
- Latest release: **v20260724.25** (GNOME desktop image) (runs-from-USB image + sysupdate transfer set)
- CI on `main`: green (lint, tests, egress, openai-compat, layer-e2e, gnome-panel-build; nightly image + A/B rollback + sysupdate; release pipeline)

**Recent (2026-07-24, after v25):**
- **Intelligence panel** in the gnome-control-center fork works (fixed the
  GNOME-50 subpage activation trap, ADR-0012) — subpages (Providers / Local
  models), model providers, and **"Sign in with Claude / ChatGPT" OAuth** via
  the lisa-remoted broker (ADR-0010/0015).
- **Lisa Assistant** — a persistent GJS chat window (ADR-0015), a second
  frontend of the overlay backend: local + cloud models, streaming, ledgered;
  **Super+C** opens it. Cloud routing enabled on the per-user inferenced
  companion.
- **/home persistence** — backed by the durable var partition so settings /
  wallpaper / SSH key survive A/B updates (boot-safe).
- **Reverse-DNS rename** — `org.lisa.*` → **`dev.lisaos.*`** (OS/daemons) +
  **`app.lisaos.*`** (apps), ADR-0016. Ships in the next release; v25 still
  carries the old names.
- **Websites** rebuilt in **Nuxt 4 + Nuxt UI** and live on staging:
  lisa-app.common.al (marketing) + lisa-dev.common.al (a **contributor portal**
  with GitHub login — needs a GitHub OAuth app before login functions — and a
  live good-first-issues board). Real domains lisaos.app/dev await DNS.

## What works (verified)

**Inference — `daemons/inferenced` (M1, §5.1):**
- Real streaming inference via a supervised `llama-server` child; `lisa
  ask` produces real tokens. Crash recovery: kill -9 the child → service
  restored in ~2 s (under the 5 s budget).
- Guided generation: OpenAI `response_format: json_schema` → liblisa
  GBNF → sampler. **1000/1000** on the sampled validation gate. Grammar
  has structural bounds (min/maxItems, min/maxLength) — unbounded rules
  let small models spiral. Server re-samples invalid guided output.
- QoS scheduler: interactive preempts background streams < 250 ms.
- `dev.lisaos.Inference1` D-Bus surface: OpenSession → (path, fd), tokens
  stream over the fd to EOF, Embed/Cancel/Close (tested over zbus p2p).
  Ships as the per-user `lisa-inferenced-dbus.service` (the hardened
  system unit can't reach the login session's bus; companion owns 7778,
  system owns 7777); a bus-loss watchdog exits the daemon so systemd
  re-registers the name instead of serving a ghost (found live on the
  iMac: session restart silently dropped the name).
- Embeddings: `/v1/embeddings` + `Engine::embed` + `lisa embed` (llama
  needs `--embeddings --pooling mean`; 1024-dim live).
- Multi-model residency: `EngineProvider`/`ModelPool` — one child per
  resident model, lazy spawn, LRU eviction; model field / D-Bus
  model_hint / /v1/models are pool-aware.
- Verified zero egress under the hardened systemd sandbox (CI).

**Model store — `daemons/modeld` (§5.2):** blake3 content-addressed
store (dedupe/verify/gc, pinned-hash ingest), hardware profiler (§8
tiers; `lisa models profile`), HTTP-Range resumable pulls. Catalog
(`models/catalog/catalog.toml`) carries one fully pinned artifact
(qwen3-0.6b-instruct-q8).

**The Ledger — `libs/lisa-ledger` (M2, §5.7.6):** append-only SQLite
(UPDATE/DELETE aborted by triggers). Enforced as the inference gate
(dataflow rule 4): a start entry precedes every generate/embed; append
failure → 503; the daemon refuses to start without a ledger. `lisa
ledger`.

**Context fabric — `daemons/contextd` (M3 core, §5.3):** per-user SQLite
(FTS5) file index with provenance tags + incremental blake3 reindex;
namespace-isolated per-app memory with zero-residual wipe. `lisa context
index/search` (searches ledgered) and `lisa memory get/set/list/wipe`.

**OS image — `os/` (M0, Track I):** mkosi Arch image builds, boots, and
demonstrates **A/B update AND rollback** in CI (boot-counting rollback +
real systemd-sysupdate into the inactive slot). swtpm in the boot check.
Track L (`os/layer/`): real packages + install/uninstall proven on
vanilla Arch (`layer-e2e`). Branded end to end: GDM greeter + session
carry the violet accent (GNOME `purple` enum — exact #6D45C9 via CSS is
open), the white Lisa wordmark on the login screen (`/etc/dconf` gdm db,
`os/mkosi/mkosi.extra`), and Rubik as the UI font (gschema override).

**Release channel — `.github/workflows/release.yml`:** GitHub Releases
ARE the sysupdate source. Weekly cron (edge channel) + on-demand;
boot-gated (no boot, no release). Each release ships the dd-able USB
image (humans) + `lisa_<ver>.root.xz` + `.efi` + `SHA256SUMS`
(machines). Devices auto-stage via `systemd-sysupdate.timer`; `lisa
update` on demand; `lisa install <disk>` streams the latest release onto
a disk (proto-installer; guided OOBE is M7).

**Flutter lane (ADR-0004 spike, macOS half):** Flutter 3.44.7 pinned.
`libs/lisa_ui` on core widgets only (tokens, LisaStreamText, ConsentChip
— widget-tested). `libs/lisa_flutter` zero-dep OpenAI-compat transport,
live round trip vs the daemon. Linux half (GTK embedder, fcitx5,
package:dbus client) pending.

**forge-harness — `libs/forge-harness` (§5.12.1 skeleton):**
plan→edit(jailed)→`dart analyze`→iterate loop with guided `{path,
content}` edits; tested against real dart analyze.

## Design direction

Owner likes **elementary OS** (restrained, humane, one visual voice).
Recorded in `docs/notes/design-direction.md`: tokens-first via the
Appendix E theme file; GNOME base kept for portal maturity; escalation
path is an own-shell-on-Mutter (Pantheon/Gala pattern), never wholesale
Pantheon. Feeds the M4 shell ADR.

## Open items / next moves

- **iMac field test:** re-imaged onto the bigger disk (2026-07-23);
  hand-syncing files is dead — fixes ship via the release channel and
  the box pulls them with `lisa update` (sysupdate). First live M4 run
  verified: extensions ACTIVE (after the shell-version fix),
  `dev.lisaos.Overlay1.UI` owned by gnome-shell, Summon → overlay shows →
  ledgered context retrieval. `lisa install <disk>` already done.
- **iMac as CI runner:** not yet registered (needs a fresh registration
  token minted at the machine); unlocks perf gates + the Flutter Linux
  spike half + real M4 desktop work.
- **M1 remainder:** LoRA hot-swap; latency budgets on reference hardware.
- **M2:** portal core landed (branch `portal-m2`, §5.5/ADR-0008):
  `dev.lisaos.Portal` session service — per-app identity, first-use
  consent (fail-closed), append-only grant store, quotas, Ledger
  attribution, revoke-kills-live-session; tested over zbus p2p incl.
  end-to-end against `dev.lisaos.Inference1`. Still open: Flatpak demo
  app on a live desktop, shell consent dialog (M4), Settings UI;
  `liblisa` SDK guided-gen samples.
- **M3 next:** embedding pipeline + hybrid ranking (sqlite-vec), file
  watchers, ACL fuzz suite, the portal Context/Memory surfaces.
- **M4:** first passes landed (branch `m4-shell`): overlay backend
  (`dev.lisaos.Overlay1`) + GNOME extension, launcher search provider
  (qalc + context lanes + **"Ask Lisa" handoff** — every query can
  summon the overlay via the frontend-owned `dev.lisaos.Overlay1.UI`
  name, Spotlight-style, prompt pre-submitted; promoted when the query
  reads like a question), Ledger app (GTK4/GJS), fcitx5-lisa proofread
  addon (ADR-0007) — pure logic unit-tested everywhere (`just
  shell-test`/`ime-test`). macOS-style summon keys: **Super+Space =
  search** (§5.7.2), **Super+Shift+Space = overlay** (§5.7.1),
  input-source switcher on Ctrl+Super+Space. First live run on the
  iMac found and fixed: metadata `shell-version` capped at 49 while
  the image ships GNOME 50 (extensions never loaded) — now declares
  50. Still need the desktop session: the §5.7 budget runs. Deferred
  within M4:
  voice v1 (§5.7.5), writing-tools layer 1 (GTK module), wlr-layer-shell
  overlay frontend, bus-action launcher lane (M5).
- **M5 (branch `m5-agentd`, §5.4, ADR-0009):** Agent Bus core landed —
  `daemons/agentd` joins the workspace with MCP-native manifest loading
  + validation (Appendix B), tool registry + discovery, the
  confirmation-tier state machine (read→silent, write→chip,
  destructive→modal) enforced **at the bus** with rule-6 provenance
  escalation (untrusted or empty chain escalates one tier, fail closed),
  Ledger attribution on every call path, and the undo journal
  (`agent-journal.db`) with manifest-declared `$input`/`$result`
  compensations behind `lisa undo`. D-Bus surface `dev.lisaos.Agent1`
  (ListTools/Discover/RequestCall/Confirm/Undo + ConfirmationRequested),
  tested over zbus p2p on macOS. `tests/injection-suite` seeded: 150 of
  the 500+ corpus, bus-layer gate green (0 unconfirmed privileged
  dispatches). Guardrail prompt at `daemons/agentd/prompts/`. Deferred
  (next slice): MCP wire transport (per-app unix socket + activation)
  behind the `Dispatcher` trait, `libs/mcp-bus`, `lisa tools/call/undo`
  CLI verbs, btrfs-snapshot file-op compensation, first-party app tools,
  model-in-the-loop injection layer — so the §5.4 demo flow is proven in
  parts, not yet end-to-end. Overlay backend swaps its direct
  `dev.lisaos.Inference1` calls for `RequestCall` when it becomes an Agent
  Bus client.
- **Hardening gaps (noted in releases):** sysupdate `Verify=no` until
  signed manifests (M1); `/etc` not overlaid yet; Arch base not yet
  snapshot-pinned in release builds (`os/repo-tools/snapshot.sh` exists).

## Working agreements that bit us (so they don't again)

- Pre-push hook (`.githooks/pre-push`, enable with `git config
  core.hooksPath .githooks`) runs fmt + clippy — an unverified push
  can't leave.
- Rust 1.97+ required (libsqlite3-sys needs `cfg_select`).
- macOS dev host is aarch64: image/systemd work is CI-only; local Arch
  container testing uses `docker.io/menci/archlinuxarm` (official image
  is amd64-only, segfaults under emulation).
- systemd-in-podman on GitHub runners needs `--privileged` (default
  seccomp kills dbus-broker → PID1 wedges).
- CI boot-checks must use the **same** root-discovery path as real
  hardware (`root=PARTLABEL`), or hardware failures stay invisible — the
  iMac's `gpt-auto-root` timeout was exactly this divergence.
- zbus must run on its `tokio` feature; grep -c exits non-zero on zero
  matches (breaks `&&` chains).
