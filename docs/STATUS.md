# Lisa OS — project status & session handoff

Living snapshot of where the build actually is, so any machine (or a
fresh Claude Code session) can pick up without reconstructing context.
`docs/PLAN.md` is still the source of truth for scope; this is the
"where are we on it" companion. **Last updated: 2026-08-02.**

## 2026-08-02 — the split (ADR-0039) and Google OAuth verified

- **The repo split happened.** Two of ADR-0006's triggers fired; three
  new org repos exist, extracted with `git filter-repo`, history
  preserved: **lisa-desktop** (`shell/*`, `ime/*` — will vendor the
  GNOME Shell fork, ADR-0038), **lisa-apps** (`apps/*` less the Rust
  `apps/notes`), **lisa-packages** (the `[lisa]` pacman index, seeded).
  **Nothing was deleted here** — this repo still builds the image
  exactly as before; #171 is the checklist for making the new path real
  (per-repo PKGBUILDs → hosted `[lisa]` → image consumes it → only then
  removal). Held triggers, on purpose: `liblisa` (no external
  consumer), Flutter lane (no shipped app).
- **Google approved OAuth brand verification** for project `lisaos`
  (2026-08-02): the "unverified app" interstitial is gone from GOA
  Google sign-in. Constraint: any new scope or consent-screen change
  re-triggers verification — the shipped scope list is frozen until we
  deliberately re-submit.
- **layer-e2e flake fixed** (contextd embed test): the HTTP stub read
  once and replied; when that read raced the client's two-write
  request, the body write hit a closed peer — BrokenPipe in CI only.
  Stub now drains a full request and counts answered requests, not
  accepted connections. 200 consecutive runs green.

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

**2026-07-26 — v20260726.34 shipped.** Four attempts, three distinct real
failures, none of them flakes:

1. **Arch moved to linux 7.1.5** and `lisa-audio-cs8409` refused to build
   against a kernel it was not pinned to. Guard 1 working exactly as
   designed — a mismatched codec module loads as *nothing*, so kernel
   drift has to be a red build rather than quiet dead speakers. Re-pinned
   (digest taken from kernel.org's `sha256sums.asc`, cross-checked against
   the 7.1.4 line that matches the existing pin).
2. **`patch_cs8409.h.diff` no longer applies** to 7.1.5 — 2 of 4 hunks
   fail in the header, `.c` still applies with offsets. Guard 3 (`-F0`)
   working. **v31 ships without CS8409** (authorized); not a regression,
   since it has never worked on hardware because it has never shipped.
   Both call sites are commented with the reason and must be re-enabled
   together; #44 reopened with the hunk numbers and a checklist.
3. **`repo-out/lisa-cli-*` did not exist on the host.** The runtime
   payload step reads the CLI package from the host, but packages are
   built into `/build/repo-out` inside a `--rm` container — a path that
   never existed outside it. That step arrived with the runtime channel
   (#52) *after* v30, so v31 was its first execution and the bug was
   latent from the day it was written. Same shape as the five silent
   no-ops from earlier this week, except this one failed loudly.

Released artifacts verified: USB 2.0 GB, root.xz 1.6 GB, UKI 152 MB, Zen
x86_64/aarch64 100/87 MB, **runtime payload 4.3 MB** (first release to
carry one), apps 76 KB, SHA256SUMS covering all seven.

**Not yet on the device.** The field iMac went off the network when the
office was left; Wake-on-LAN did not raise it. The image masks suspend
deliberately (`sleep.target` → `/dev/null`, `IdleAction=ignore`, GNOME
power schema `'nothing'`) because that machine's amdgpu cannot resume, so
either it was powered off by hand (benign) or suspend fired despite the
masking (a bug). `systemctl is-enabled sleep.target` and the journal will
say which. **Still unverified on hardware: the boot splash and the
speakers** — both need a human in the room.

Pre-update diagnostics that did run, all clean: `systemd-pull` and
`systemd-sysupdate` present, `libcurl.so.4` present **with all its own
dependencies resolving** (the actual dlopen failure mode behind #45), CA
bundle present, both transfers carrying `ProtectVersion=`, zero failed
units, and `/etc/NetworkManager/system-connections` symlinked onto `/var`
with autoconnect on — so Wi-Fi will survive the slot swap.

CI caching (pacman + pinned kernel tarball) landed but is **unproven**:
the first run populates rather than restores, and it measured 25 min
against a 22 min baseline. The next release is the real test; if it does
not pay off the honest next step is `sccache`, not paid runners.

**2026-07-26:**
- **The vision got a spine** (ADR-0030, ADR-0031, ADR-0032; `docs/VISION.md`).
  A day of guardrail work and a long design conversation produced three
  things worth treating as core rather than as chat:
  1. **"Probabilistic reasoning inside, logical guardrails outside"** is now
     the governing principle (CLAUDE.md rule 6a), with a testable
     invariant — *the boundary must not be reachable from inside* — and
     the corollary that **the owner is outside it too**. ADR-0029 had made
     `Deny` absolute, which put a guardrail between a person and their own
     machine; that was a category error. `lisa guard list|allow|forbid`
     now lets the machine's owner relax a rule out-of-band, where no tool
     call can reach it, and a relaxed rule *warns* rather than going
     silent. `lisa suggest` honours relaxations because a human is
     present; the forge loop deliberately does not, because nobody is
     watching it.
  2. **Make and serve** — Lisa builds the artifact and serves it from your
     hardware under your domain. ChatGPT/Claude make but don't serve;
     Vercel/CapRover serve but don't make; v0/Lovable do both on *their*
     infrastructure. The closure is the differentiator. Server mode, the
     private (WireGuard) and public (domain + ACME) edges, Cockpit for
     management vs our own surface for use, and publishing as a
     confirm-tier ledgered act are specified in ADR-0031 — **proposed,
     no code, sequenced after v31** and gated on #55 + the injection
     suite, because publishing is the first capability whose failure
     harms someone other than the owner.
  3. **Construct and Lisa are one idea at two levels** (ADR-0032). They
     share a *contract* — manifest, provenance vocabulary, Ledger event
     shape, design tokens — not code; a Vue Space does not port to a
     Flutter app and never will. The contract is defined in Lisa because
     a constrained spec relaxes onto a platform easily and a permissive
     one never grows enforcement.
  Also recorded: what the Forge may **produce** is now a security
  boundary sequenced by blast radius (GUI apps → CLI → system units, the
  last gated on Landlock #53), and Lisa's two UI stacks have a stated
  line — GJS for shell surfaces, Flutter for applications.
- **Hard guardrails for agent actions** (ADR-0029, new crate
  `libs/lisa-guard`, issues #53–#58). An audit of the two agent execution
  surfaces found the Agent Bus genuinely guarded — tiers, provenance
  escalation, ledger-before-dispatch, undo journal — and the **forge
  harness, where autonomous execution actually happens, guarded by a doc
  comment**. Two working escapes, not hypotheticals: `run_command`
  pivoted to a full shell via `find . -exec sh -c '<anything>' \;` (every
  token is a plain relative name, so the absolute/`..` check waved it
  through), and the path jail was blind to symlinks (`resolve` never
  canonicalized, `fs::write` follows links, so a link inside the project
  wrote outside it). Chained, those two are a complete escape using only
  allowlisted tools. Policy now lives in one deterministic crate outside
  the model — no prompt text, no heuristics — with a `Deny` class no
  confirmation or `--yes` can override. `lisa suggest` is screened
  *before* printing, since stdout is what the Ctrl+G hook types into the
  user's shell buffer. Merge gate: 49 destructive attempts, none allowed;
  17 everyday commands unobstructed. **Stated limit:** none of this
  confines a subprocess — `run_tests` runs `cargo test` over
  model-written source, which executes `build.rs` as the user. Landlock
  closes that (#53).
- **…and then an adversarial review broke it in eight places** (#59–#66,
  all fixed in `26e8888` except the narrowed remainder of #66). The
  corpus had been green because it listed every attack in its plainest
  spelling: `/bin/rm -rf /`, `rm${IFS}-rf${IFS}/`, `$'\x72\x6d' -rf /`,
  `( rm -rf / )`, `eval "rm -rf /"`, `/usr/../etc` and
  `cargo --config '…runner=["/bin/sh",…]'` all returned `Allow`. The
  worst two: `cargo --config` is `find -exec` wearing a build tool
  (proven end-to-end — the injected shell ran and wrote outside the
  project), and check-then-write lost a symlink race **18,599 times out
  of 20,001**. Fixes: basename reduction, expansion normalization,
  compound-command splitting, a fail-closed `shell.unreadable` verdict,
  per-program policy with subcommand allowlists, position-aware path
  checking, and `O_NOFOLLOW` writes. Corpus 49 → 75 denied entries.
  ADR-0029 gained a review-round section — including that its §1 claimed
  a mitigation the code never had.
- **Round 2 found eleven more** (#67–#77, all closed in `4502a52`), each
  one a round-1 fix being correct for exactly the spelling that prompted
  it: `env -u FOO rm -rf /`, `&>`, `sh <<<`, `python3 -c 'os.system(…)'`,
  `rm -rf $TARGET`, `/etc/systemd/system`, `dart pub global activate`.
  **Two rounds, nineteen findings, in code whose entire claim is that it
  cannot be talked past** — that is the honest character of shell-string
  parsing. Three fixes changed strategy rather than adding a case, on the
  principle that enumerating the dangerous spellings of a shell is a
  losing game: wrapper option grammars are no longer modelled (every word
  after a wrapper is judged as a candidate program), inline source in a
  language the reader does not speak is refused rather than
  approximated, and unresolved `$…` is refused in target position as well
  as program position. Corpus 75 → 105 denied, 17 → 29 must-allow — the
  must-allow half grew on purpose, since each round also produced a false
  positive. If it leaks a third time the answer is to stop parsing shell:
  have `lisa suggest` emit structured argv the guard can judge exactly.
- **Round 3 found ten more** (#78–#87, all closed across `3821dc0` and
  `3e1e913`) — and **one of them was a regression a round-2 fix
  created**: `cargo -- evil-plugin` runs `cargo-evil-plugin` from PATH,
  because the `--` split added for a *false positive* skipped everything
  after `--`. Arbitrary program execution on the surface with no human,
  no ledger and no confinement; landed on its own ahead of the rest.
  Four more were classes nothing before them anticipated (the decoder
  feeding the tokenizer, `<>`, globs in target position, comments).
  Corpus 105 → 128 denied, 29 → 41 must-allow.
  **8 → 11 → 10 findings: flat, not converging — so the strategy
  changed** (issue #88). `check_command` guards argv and leaked 4 times
  in 3 rounds; `check_shell_line` guards an arbitrary shell string and
  leaked 15. The difference is the input, not the effort, so `lisa
  suggest` moves to emitting **structured argv** that the exact,
  bounded `check_command` logic can judge, with the shell string
  rendered only for display. Every fix in that file has been correct and
  every fix has left the next spelling open — which is what a guardrail
  built by enumeration looks like from the inside, and why the corpus is
  called a floor: it was green after every round.
- **Zen browser moved to the apps channel** (ADR-0023 phase 1, issue #51).
  `zen-browser` is now a split build: `zen-browser-launcher` (the
  `.desktop`, hicolor icons and a `/usr/bin/zen-browser` resolver) stays in
  the image forever, while the `/opt/zen` payload ships as
  `lisa-zen_<ver>_<arch>.tar.zst` for **both** architectures and installs
  under `/var/lib/lisa/apps/payloads/zen`. `lisa apps` grew channels
  (`update`/`rollback [channel]`, new `sync`), per-arch asset selection,
  streamed downloads and version pruning; `lisa update` pre-fetches
  payloads before staging a slot and `lisa-apps-sync.timer` retries until
  they land. **This release still bakes `/opt/zen`** — one deliberate
  overlap so no device can boot a slot whose browser its old `lisa update`
  never fetched; the next release deletes the one `Packages=` line.
- **The ADR-0023 size premise was wrong and is corrected.** Measured from
  the pinned upstream tarballs: `/opt/zen` is **363 MiB** (x86_64) /
  328 MiB (aarch64), not ~1.5 GiB — 726 MiB across the A/B pair, ~90 MiB
  off every release download. So the populated root goes ~8.3 → ~7.9 GiB
  and **phase 3's 7 GiB slot target does not fit**; release.yml now records
  the real populated root in every release's job summary so that call is
  made on a measurement.

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
  within M4: writing-tools layer 1 (GTK module), wlr-layer-shell
  overlay frontend, bus-action launcher lane (M5).
- **Voice v1 (§5.7.5, ADR-0011) — landed 2026-07-31, untried on
  hardware.** For a week this was code that could not run: Arch packages
  neither engine, so no device had one, and `lisa say` could never have
  produced a sound (a voice path nothing creates, a flag piper does not
  have, and success returned on every failure). Now: `whisper.cpp` and
  `piper` are packaged and in the image lane, a redistributable voice is
  pinned (LibriTTS-R, CC BY 4.0 — the well-known alternatives are a
  signed licence form or non-commercial), `lisa listen` captures,
  `lisa ambient once` runs the whole loop from the microphone, and
  push-to-talk holds a key in the shell via `dev.lisaos.Voice1`. Every
  transcription is ledgered as `voice.transcribe`.
  Verified as a round trip in a container with the build trees deleted —
  piper says a sentence, whisper hears it back word for word.
  **Both halves are also proven on the reference iMac18,2 (2026-07-31)**,
  by shuttling audio rather than waiting for a release: a recording made
  by the iMac's own microphone (CS8409/CS42L83, the ADR-0024 codec) went
  through the packaged whisper and came back "Hello? Can you hear me?",
  and a sentence synthesized by the packaged piper played out of the
  iMac's speakers and was heard. Mic gain is sane (100%, +12 dB, 0.01%
  full-scale samples). What is still **untried on hardware is the key
  itself**: push-to-talk needs the two packages installed, so it waits
  for the next release. ARM gets speech in and not out (no onnxruntime on Arch
  Linux ARM). The ambient loop (VAD, ring buffer, hard mute, wake word)
  is ADR-0011 stage 3 and has not started; nothing in the repo records
  unprompted.
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
