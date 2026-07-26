# Lisa OS — project status & session handoff

Living snapshot of where the build actually is, so any machine (or a
fresh Claude Code session) can pick up without reconstructing context.
`docs/PLAN.md` is still the source of truth for scope; this is the
"where are we on it" companion. **Last updated: 2026-07-25.**

## TL;DR

Three days from planning doc to a **bootable, self-updating OS with a
public release channel**. The inference substrate (M1) is functionally
complete; M2 (Ledger) and M3 (context fabric) have working cores. Every
claim below is enforced by CI on `main`, not aspirational.

- Repo: **github.com/Lisa-AgenticOS/lisa-os** · License: GPL-2.0-only (ADR-0005)
- Latest release: **v20260725.27** — first release whose sysupdate transfers
  carry `ProtectVersion=%A` (the issue-#20 booted-slot guard), gated by the
  nightly's 3-version regression test
- CI on `main`: green (lint, tests, egress, openai-compat, layer-e2e, gnome-panel-build; nightly image + A/B rollback + sysupdate; release pipeline)

**2026-07-25 early-morning (autonomous loop, continued):**
- **Local M4 test rig**: the CI-built aarch64 image boots on the dev Mac
  (QEMU + HVF, ~30 s to desktop) and replaced the broken iMac as the
  feature-test device. Verified there: ADR-0018 PARTLABEL mounts, ADR-0019
  runtime repart (grow disk → var+home split created), never-suspend
  masking, `lisa ask` at ~3 s round-trip, the **overlay live end-to-end**
  (D-Bus `Summon` → streamed local answer, chips, ledgered badge) — with the
  whole Rust stack built natively inside the VM. QMP screendumps + key
  injection give visual "computer-use" driving.
- **Harness chase, three substrate fixes from one test session**: the Dart
  verifier passed vacuously on an empty scaffold (issue #29 — bare "done"
  converged with zero files; fixed, empty project is now findings); a cold
  llama-server load slower than the fixed 60 s window surfaced as a spurious
  503 (now `llama.health_timeout_secs`, default 300); and llama-server was
  spawned without `--jinja`, so OpenAI `tools` were ignored and every agent
  turn degraded to plain text (now fixed — tool calling actually reaches the
  model).
- **Issue #16 (dual-disk ambiguity) landed in three layers**:
  `lisa-boot-disk-generator` pins var/home/efi to the booted disk (verified
  on the rig), `lisa install` regenerates copied btrfs fsids with
  `btrfstune -m`, and `lisa update` refuses transfer configs lacking
  `ProtectVersion=` (escape hatch: `LISA_UPDATE_ALLOW_UNPROTECTED=1`).
  Open remainder: initrd-side `root=` scoping.

**Overnight 2026-07-24→25 (autonomous loop):**
- **A/B update emergency-mode bug root-caused in three layers** (ADR-0018):
  per-build /var identifiers → mount by PARTLABEL via fstab; the nightly's
  byte-copy duplicate btrfs fsid → btrfstune -m; and systemd-gpt-auto's
  phantom machine-id-keyed var.mount racing the fstab-generator →
  `systemd.gpt_auto=no`, with /var, /home, and /efi as explicit fstab
  PARTLABEL mounts. The ab-sysupdate scenario now lives in
  `.github/actions/ab-sysupdate` and runs in BOTH lanes — nightly and,
  before publishing, release.yml against the artifact devices receive
  (issue #47; it used to gate only the nightly image, which is a
  different build).
- **On-device (field iMac, .22): the Assistant chat is verified end-to-end** —
  streamed tokens, ledgered (`inference.generate/complete`), window renders
  with a live model picker. Two shipping bugs found live and fixed: the model
  pool assigned llama children the daemons' own ports (kernel-allocated free
  ports now), and GJS TextDecoder lacks `{stream:true}` (boundary-safe
  incremental decode now). Auto-suspend disabled and masked in-image — the
  machine cannot resume from suspend (amdgpu), so it never sleeps.
- **Apps wave:** Assistant Stop/export/persisted history (via the new
  contextd), Ledger free-text search, Notes `search_notes`, and the §5.8
  **Terminal integration** (`lisa explain`, `lisa suggest`, Ctrl+G with a
  review gate). Plus: true SSE streaming through the remoted broker,
  double-tap-Shift summon (fcitx5), `dev.lisaos.Context1` (M3's missing
  IPC surface), a dedicated /home partition for fresh installs (ADR-0019),
  and the **app-update channel** (`lisa apps update`, ADR-0020 — app updates
  without an image release).
- **Apple Silicon:** BOTH halves proven — the full Track L e2e passes
  natively on aarch64 (podman/ALARM), and the ARM64 Track I image now
  **builds and boots in CI** (aarch64-image.yml, ALARM base, ADR-0021).
  That image is no longer bare: `aarch64-image.yml` builds the **same**
  package set as the x86_64 release lane natively on ARM — the `lisa-*`
  split packages, llama.cpp, the forked gnome-control-center, and
  zen-browser (upstream does ship an arm64 tarball; the earlier
  "x86_64-only" note in ADR-0021 was an unchecked assumption) — folds
  them into the image and asserts the binaries are there and aarch64
  before the boot check (issue #28). The only remaining per-arch delta is
  the kernel (`linux-aarch64` vs `linux`). Unproven until the workflow
  runs: that the three source packages compile on ARM.

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
(`models/catalog/catalog.toml`) carries six fully pinned artifacts:
whisper-base-en, gemma-3-1b-it-q8, qwen3-0.6b-instruct-q8,
qwen3-1.7b-instruct-q8, qwen3-4b-instruct-q4, and
nomic-embed-text-v1.5 — each downloaded and blake3-verified (issue #7).

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

**Flutter lane on-device (issue #37, ADR-0027):** `lisa forge --setup`
installs the pinned 3.44.7 SDK to `/var/lib/lisa/flutter` — sha256-pinned
tarball on x86_64, and on **aarch64** a commit-pinned checkout of the same
release (Google publishes no arm64 tarball, but does publish the arm64
Dart SDK and `linux-arm64` engine artifacts; the pinned commit is the id
Google's own manifest carries). `lisa forge --build` / `--run` generate the
Linux runner from the SDK template, `flutter build linux --release`,
install the bundle to `/var/lib/lisa/forge/apps/<app-id>/bundle`
(stage-then-rename, one rollback generation, ADR-0023) and write a
`~/.local/share/applications/app.lisaos.forge.<pkg>.desktop` entry.
**Open:** the Track I image ships no clang/cmake/ninja, so building on an
immutable device needs the toolchain decision in ADR-0027; Track L and dev
hosts work today. The end-to-end `flutter build linux` has never run — no
Linux desktop here.

**Skills (ADR-0025 phase 4 groundwork):** `skills/<name>/SKILL.md` in the
repo, `/usr/share/lisa/skills` on device; `lisa skills list|show` resolves
`$LISA_SKILLS_DIR` → `~/.local/share/lisa/skills` → the packaged set. First
skill shipped: `build-lisa-ui-app`.

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
- **Boot splash on hardware (ADR-0025, issue #26):** the initrd now
  carries `amdgpu` + its firmware (and `virtio_gpu` for QEMU) — ADR-0017's
  `simpledrm` entry matched nothing, since simpledrm is built into Arch's
  kernel, so the iMac booted black from the Apple logo to GDM. Mechanism
  landed and asserted in the nightly; **appearance still unconfirmed on
  the device** — needs one graphical boot after the next release.
- **Nothing we wrote ever reached the initrd (issue #50, ADR-0028):**
  `os/mkosi/mkosi.initrd/` looked like a mkosi convention and is one in no
  version of mkosi — the default initrd is an internal sub-image built only
  from mkosi's own resources. Both halves of that directory were dead, so
  ADR-0017's Plymouth (the *package*, not just its config) and ADR-0022
  phase 2's rescue root resolver were never in the initrd. Files now ride a
  cpio through `$ARTIFACTDIR/io.mkosi.initrd` (`os/mkosi/mkosi.finalize` +
  `os/mkosi/initrd-overlay/`), packages through `InitrdProfiles=`, and the
  nightly asserts the payload inside the built UKI. **Unconfirmed until the
  next nightly:** this is the first build where Plymouth is genuinely in
  the initrd.
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
