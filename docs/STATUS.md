# Lisa OS — project status & session handoff

Living snapshot of where the build actually is, so any machine (or a
fresh Claude Code session) can pick up without reconstructing context.
`docs/PLAN.md` is still the source of truth for scope; this is the
"where are we on it" companion. **Last updated: 2026-07-22.**

## TL;DR

Three days from planning doc to a **bootable, self-updating OS with a
public release channel**. The inference substrate (M1) is functionally
complete; M2 (Ledger) and M3 (context fabric) have working cores. Every
claim below is enforced by CI on `main`, not aspirational.

- Repo: **github.com/Lisa-AgenticOS/lisa-os** · License: GPL-2.0-only (ADR-0005)
- Latest release: **v20260722.4** (runs-from-USB image + sysupdate transfer set)
- CI on `main`: green (lint, tests, egress, openai-compat, layer-e2e; nightly image + A/B rollback + sysupdate; release pipeline)

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
- `org.lisa.Inference1` D-Bus surface: OpenSession → (path, fd), tokens
  stream over the fd to EOF, Embed/Cancel/Close (tested over zbus p2p).
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
vanilla Arch (`layer-e2e`).

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

- **iMac field test:** boot v20260722.4 USB on the Intel iMac (root now
  found via `root=PARTLABEL`; USB-HID in the initrd). Ethernet works;
  Wi-Fi likely not (Broadcom firmware). `lisa install <disk>` erases
  Ubuntu — owner's call.
- **iMac as CI runner:** not yet registered (needs a fresh registration
  token minted at the machine); unlocks perf gates + the Flutter Linux
  spike half + real M4 desktop work.
- **M1 remainder:** LoRA hot-swap; latency budgets on reference hardware.
- **M2:** portal core landed (branch `portal-m2`, §5.5/ADR-0008):
  `org.lisa.Portal` session service — per-app identity, first-use
  consent (fail-closed), append-only grant store, quotas, Ledger
  attribution, revoke-kills-live-session; tested over zbus p2p incl.
  end-to-end against `org.lisa.Inference1`. Still open: Flatpak demo
  app on a live desktop, shell consent dialog (M4), Settings UI;
  `liblisa` SDK guided-gen samples.
- **M3 next:** embedding pipeline + hybrid ranking (sqlite-vec), file
  watchers, ACL fuzz suite, the portal Context/Memory surfaces.
- **M4:** first passes landed (branch `m4-shell`): overlay backend
  (`org.lisa.Overlay1`) + GNOME extension, launcher search provider
  (qalc + context lanes), Ledger app (GTK4/GJS), fcitx5-lisa proofread
  addon (ADR-0007) — pure logic unit-tested everywhere (`just
  shell-test`/`ime-test`); live verification and the §5.7 budget runs
  still need a Linux desktop session (the iMac). Deferred within M4:
  voice v1 (§5.7.5), writing-tools layer 1 (GTK module), wlr-layer-shell
  overlay frontend, bus-action launcher lane (M5).
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
