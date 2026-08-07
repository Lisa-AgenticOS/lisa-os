# Architecture Decision Records

Any deviation from `docs/PLAN.md` — a dead library, a changed API, a
superseded model, a better idea — gets an ADR *before* the code changes.
Never silently improvise (PLAN §0.4).

**Read `docs/VISION.md` first.** It is one page and it says what Lisa is,
what is true today, and what is decided but unbuilt. These files are
the reasoning behind that page, and they are a *historical record*: an
ADR says what we chose and why, in the words of the day we chose it.
Only its status line is kept current.

## The table below is generated

An ADR records a decision, not a delivery. "Accepted" says we chose
something; on its own it says nothing about whether the thing exists —
so every status line names where the decision actually stands, and this
table is derived from those lines by
`os/repo-tools/build-adr-index.py`, which `just lint` runs in `--check`
mode.

It is generated because the hand-written version drifted in the way that
costs the most: it claimed "36 of the 37 records below carry no status
line" when there were 50 records and all 50 had one, and its
what-is-built table stopped at ADR-0038, so absence read as "not built"
when it meant "nobody looked". A page describing the ADRs has to be
DERIVED from them — including its own count, which is why no number is
written by hand above this line. (The prose here said "50 files" while
the generated table below said 54: the paragraph warning about stale
counts had a stale count in it. Fixed 2026-08-05.)

## Statuses are checked, not trusted

A status line is prose, and prose cannot go red. Until 2026-08-06 nothing
here could tell a true status from a false one, and an audit that day found
rot in **both** directions across the set — ADR-0029 listed two open items
that had both shipped, ADR-0015 said the Assistant had no write tier and no
memory when it had both, ADR-0036 said "nothing is implemented" while its
shell tool shipped with all four of its conditions.

So an ADR may carry a `- **Claims:**` block, and one whose status asserts
implementation **must**:

```
- **Claims:**
  - `path:libs/forge-harness/src/shell_tool.rs` — §6's shell tool
  - `symbol:check_shell_line@libs/forge-harness/src/shell_tool.rs` — condition 2
  - `absent:libs/lisa_ui` — the name is still free
  - `nomatch:zbus@daemons/modeld/Cargo.toml` — modeld is still not a bus daemon
```

Four kinds, two pairs: `path`/`absent` for existence and `symbol`/`nomatch`
for content. The second of each pair is what catches a status that
*understates* — the kind that gets working code deleted to match a document.
`build-adr-index.py --check` evaluates every claim on every run of
`just lint`, and a missing file fails a claim rather than skipping it.

What a claim proves is narrow and worth stating: a `symbol:` claim is a
static read of the source, so it proves the tree says so, not that the
binary runs. It is deliberately not a test runner — a gate that skips itself
when a build is missing is green on an empty checkout, which is the failure
mode every gate audited that day had.

Removing claims is allowed; removing them *quietly* is not. The counts are
in the generated table below and the floors are named constants in the
script, so a shrinking corpus costs a line somebody has to defend.

Read it as: **what would a person find on a device today.** Entries are
expected to name their evidence — an earlier table listed ADR-0025 among
the unbuilt because someone grepped for `Agent1` and found tooltips
instead of following `RunSync` to `dev.lisaos.Harness1`. A table of what
is built is worth less than nothing if its entries are inferred from a
grep.

<!-- BEGIN GENERATED INDEX — os/repo-tools/build-adr-index.py; edit the ADRs, not this table -->

**62 records** — 5 superseded in part, 27 accepted and partly executed, 2 accepted with no code yet, 25 accepted and done, 3 proposed.

**164 machine-checked claims across 58 records.** The `Checks` column is how many artifacts each ADR names that must exist (or must still be absent) for its status to be true; `build-adr-index.py --check` verifies every one of them, and a status that asserts implementation without any is a red build.

| ADR | Decision | Status | Checks | Where it actually stands |
|---|---|---|---|---|
| [0001](0001-arch-immutable-base.md) | Fork Arch Linux; ship an immutable, atomic, image-based OS via mkosi | accepted | 3 | the mkosi/UKI A/B image builds, boots, and demonstrates update *and* rollback in CI. |
| [0002](0002-rust-zbus-axum.md) | Rust with zbus + axum for system daemons | accepted | 3 | every daemon that serves a bus is Rust on zbus, with axum where there is HTTP (`inferenced`, `remoted`). Corrected 2026-08-06: the word was "every", and `daemons/modeld` has never been on zbus — it declares neither zbus nor axum and ships no unit, because it is the content-addressed store `lisa models` links rather than a running service. CLAUDE.md rule 5 already carried the same correction. |
| [0003](0003-two-track-delivery.md) | Two-track delivery — Lisa Layer first, immutable image as the product | accepted | 3 | both tracks ship: Track L installs onto stock Arch from the signed `[lisa]` index, Track I is the released image. |
| [0004](0004-flutter-lane-forge.md) | Flutter app lane + the Forge | superseded in part by ADR-0047 | 2 | ADR-0047 — the lane split is no longer in force: GJS + GTK4/Adwaita is the default for Lisa's apps and for Forge output, and Flutter is parked. The Forge itself (PLAN §5.12.1) stands, and the spike findings at the foot of this file stand as history. |
| [0005](0005-gpl2-license.md) | License the project GPL-2.0-only | accepted | 2 | — |
| [0006](0006-monorepo-staged-extraction.md) | Monorepo with staged extraction | accepted | 4 | extended, not superseded, by ADR-0039: none of this ADR's own four triggers has fired; the two that fired are ones it could not have contained. |
| [0007](0007-fcitx5-addon-cxx.md) | fcitx5-lisa is a C++ addon (thin), logic stays on the daemon side | accepted | 3 | the addon builds against fcitx5 in CI and its protocol logic is unit-tested (`just ime-test`). |
| [0008](0008-portal-standalone-service.md) | The Lisa portal is a standalone session service, consent stays in the shell | accepted | 3 | installed on the device since v20260730.55 (#153). |
| [0009](0009-agent-bus-core.md) | Agent Bus core — D-Bus surface, tier enforcement at the bus, staged MCP transport | accepted, partially executed | 3 | the bus, the tier state machine, provenance escalation and the undo journal are live in `daemons/agentd`, and MCP genuinely rides per-app unix sockets (`libs/mcp-bus`, `McpDispatcher`), not in-process dispatch. Open: socket activation (`mcp.activatable` is declared and unimplemented), and the §5.4 acceptance flow end to end. |
| [0010](0010-remote-providers.md) | BYO remote model providers via a dedicated egress broker (`lisa-remoted`) | accepted | 3 | `lisa-remoted` is the sole egress broker for provider traffic; PKCE state fixed (#110). A live sign-in on the device is still the outstanding confirmation. |
| [0011](0011-ambient-assistant.md) | Lisa Ambient — the always-on, wake-word-free assistant | accepted, partially executed | 4 | corrected 2026-08-04, because "NO implementation" was read off the wrong subject. The primitives exist and are proven on the reference iMac (2026-07-31): `lisa listen`/`transcribe` on packaged whisper.cpp, `lisa say` on packaged piper, `lisa ambient classify`, push-to-talk over `dev.lisaos.Voice1`, both engines installed by the image lane and both voice models pinned in the catalog. What does not exist is this ADR's actual subject — the always-on loop (VAD, ring buffer, hard mute, addressed-intent classification running unprompted) and the `voiced` daemon. Nothing in the repo records unprompted (#158). |
| [0012](0012-gnome-control-center-lisa-panel.md) | A native "Intelligence" panel in a forked gnome-control-center | accepted | 2 | the panel ships in the image and runs on the device (v25+), including provider OAuth. ADR-0048 §3 puts `gnome-control-center-lisa` on a path to retirement in favour of `shell/settings`; nothing is removed yet. |
| [0013](0013-harness-intents-and-coding-agent.md) | The Lisa harness — Siri-style intents + a Claude-Code-level coding agent, on the existing substrate | accepted, partially executed | 3 | Sessions, Skills, Memory and the policy layer shipped in `libs/harness-core` and `lisa forge` runs on them. Remaining pillar: Crons, deliberately last (ADR-0025 phase 5). |
| [0014](0014-lisa-ui-material-fork.md) | lisa_ui becomes the kit Lisa apps import — Material-backed now, vendored fork later | superseded in part by ADR-0047 | 1 | ADR-0047, and again by ADR-0056 — `lisa_ui` keeps the name and the role, but the toolkit it named is gone and **the library does not exist**. Corrected 2026-08-06: this line read "it is now the shared GJS/GTK4 library", which claimed as built the one thing ADR-0047 records as not built. The Dart lane was deleted on 2026-08-06 (`d1bdc18`), `libs/lisa_ui` and `libs/lisa_flutter` are both absent, and ADR-0056 is the record of what `lisa_ui` will be when it is written. The argument for owning the kit stands; nothing implements it. |
| [0015](0015-assistant-app.md) | a persistent Assistant chat window — the surface that makes the model usable | accepted, partially executed | 4 | the window ships, streams and is ledgered on the device, and read-tier tools reach it through `dev.lisaos.Harness1`. Corrected 2026-08-06: this line said "no write tier and no memory across conversations (#157)" and **both had shipped** — memory is `shell/assistant/lib/memory.js` against `MemoryList`/`MemoryForget` on `dev.lisaos.Harness1` (`daemons/harnessd/src/dbus.rs`), and the write tier is offered and gated by `bus_tools::write_tier_allowed` with agentd enforcing it (#216), not by any filter in a GJS window. Open: the device acceptance of the write path. **Widened by ADR-0053:** this ADR's "one headless backend, many thin frontends" is the pattern Lisa Server extends over a network — the GJS window becomes one frontend among several (web, API, mobile) rather than *the* Assistant, which makes the backend contract public API and something to version deliberately. |
| [0016](0016-reverse-dns-naming.md) | reverse-DNS identifiers move to the real domains (dev.lisaos.* / app.lisaos.*) | accepted | 2 | — |
| [0017](0017-plymouth-in-initrd.md) | Plymouth + the lisa theme move into the mkosi-initrd | accepted, partially executed | 2 | the `simpledrm`-only display clause is amended by ADR-0026 and the delivery mechanism by ADR-0028. Plymouth is genuinely in the initrd and asserted in the nightly; the splash→desktop handoff has never been seen on hardware (#26). |
| [0018](0018-var-pinned-partuuid.md) | /var is mounted by partition LABEL, not by UUID | accepted | 3 | — |
| [0019](0019-dedicated-home-partition.md) | a dedicated /home partition on fresh installs, weight-split with var | accepted | 2 | — |
| [0020](0020-app-update-channel.md) | app updates decoupled from the OS image | accepted | 2 | `lisa apps update/rollback/sync` ship and devices pull payloads. The channel is monolithic, which per-app store versioning will have to change (#239). |
| [0021](0021-aarch64-lane.md) | aarch64 image lane on an Arch Linux ARM base | accepted, partially executed | 3 | the aarch64 image builds and boots in CI on an ALARM base. Corrected 2026-08-06: "with the same package set as x86_64" stopped being true when ADR-0038's Shell fork landed — `mkosi.conf.d/aarch64.conf` installs **stock** `gnome-shell` and says so, because `lisa-desktop-shell` is `arch=(x86_64)`, so this lane ships a desktop that is not Lisa Desktop. No aarch64 image has been published, and ARM has speech in but not out (no onnxruntime on Arch Linux ARM). |
| [0022](0022-rescue-boot-path.md) | A user-survivable rescue boot path | accepted, partially executed | 3 | phases 1–3 are implemented and proven by execution (phase 1 self-repair and phase 3's shell on an unbootable machine are green in ab-recovery; phase 2's resolver refuses a half-written slot on its GPT type in ab-interrupted-transfer). Still missing: the boot ENTRY — the resolver works, nothing offers it in the menu (#23). |
| [0023](0023-slim-core-var-grows.md) | Slim core, /var grows — apps and heavy payloads leave the image | accepted, partially executed | 2 | phase 1 complete and device-verified (the baked `/opt/zen` left both image lanes, #89). Phase 2 (installer pre-pull) not started. Phase 3 (slot shrink) was tried at 7G and reverted the same day: the `du` figure this ADR reasoned from is not the quantity that governs slot size — see "Phase 3, attempted". |
| [0024](0024-apple-cs8409-out-of-tree-codec.md) | ship an out-of-tree CS8409 codec module for Apple speakers | accepted, partially executed | 2 | the module is packaged and re-pinned at every kernel bump, and a mismatched pin fails the build loudly by design. The reference iMac's speakers have still never made a sound through it (#44). |
| [0025](0025-one-agent-loop.md) | One agent loop — the Lisa harness | accepted, partially executed | 4 | one loop exists: the Assistant runs on `dev.lisaos.Harness1` and reaches the Agent Bus through `bus-tools`. Skills carry an enforced tool allowlist (`Skill::allowed_by`, `libs/harness-core/src/skill.rs`), and — corrected 2026-08-06 — the one shipped skill now populates it: `skills/build-lisa-app/SKILL.md` carries a `tools:` line, which is what #245 was. Phase 5 (Crons) is not started, and nothing in the tree schedules an agent run. |
| [0026](0026-native-drm-in-initrd.md) | The native GPU driver + its firmware ride the initrd | accepted | 2 | — |
| [0027](0027-flutter-on-device-aarch64-and-forged-app-launch.md) | the Flutter lane on-device — aarch64 SDK, and how a forged app gets launched | superseded in part by ADR-0047 | 3 | ADR-0047 — #37 is closed won't-do, so the on-device Flutter SDK, the aarch64 pin and the forged-app build/launch path are parked with the lane. §3 (Skills live in `skills/<name>/SKILL.md`, installed to `/usr/share/lisa/skills`) is unaffected and in force. |
| [0028](0028-initrd-overlay-mechanism.md) | Files reach the default initrd through `io.mkosi.initrd`, not `mkosi.initrd/` | accepted | 3 | — |
| [0029](0029-hard-guardrails-for-agent-actions.md) | Hard guardrails for agent actions — policy outside the model | accepted, partially executed | 4 | phases 1–3 implemented, with three adversarial review rounds folded in (corpus 49 → 128 denied). Corrected 2026-08-06: this line named two open items and **both had shipped** — Landlock confinement (#53) landed 2026-07-31 in `libs/forge-harness/src/confine.rs` with `tests/confinement.rs` behind it, and `lisa suggest` has returned structured `{program, args}` steps since 2026-07-31 (`cli/lisa/src/terminal.rs`, #88). §"Phase 3, implemented" in this file recorded the first one on 2026-08-02 and the status line was left behind, which is the exact failure that section warns about. Open: the network, which Landlock 0.4 does not reach. |
| [0030](0030-the-guardrail-boundary.md) | The guardrail boundary — probabilistic inside, logical outside | accepted | 2 | the principle has teeth rather than prose: #145 and #55 were both closed against it, and `lisa guard list\|allow\|forbid` is the owner's out-of-band relaxation, where no tool call can reach it. |
| [0031](0031-server-mode-two-edges-and-artifact-publishing.md) | Server mode, the two edges, and artifact publishing | superseded in part by ADR-0053 | 1 | ADR-0053 — still proposed, still no code (no `serverd`, no `lisa serve`, neither edge exists). ADR-0053 promotes §1's "server mode is a flavor chosen at install" into a **product** with its own surfaces and a sequencing ladder, and re-decides §2's management/use split: the first server surface is the Assistant as an API/web frontend of the existing backend, not a Cockpit module. §3 (the two network edges) and §4 (artifact publishing) stand, and are what ADR-0053's network-identity work builds on. |
| [0032](0032-construct-and-lisa-one-contract-two-levels.md) | Construct and Lisa — one contract, two levels | proposed | — | design only, no code. The shared contract (manifest, provenance vocabulary, Ledger event shape, tokens) is defined on the Lisa side only. |
| [0033](0033-identity-comes-from-the-transport.md) | Identity comes from the transport, not the message | accepted, partially executed | 3 | `libs/lisa-peer` is the primitive, and agentd, contextd, harnessd, remoted and the portal all link it. The sweep for the remaining callers is unfinished; the rule itself is CLAUDE.md 6b. |
| [0034](0034-lisa-dev-user-scope-tooling.md) | `lisa dev` — developer tooling in the user's home, rootless | accepted, partially executed | 3 | phases 0, 1 and 2 are implemented and proven by execution on a real rootless podman: `lisa dev install\|remove\|list\|shell\|reset\|doctor\|check`, a /home disk guard that measures the container store's own filesystem, shims that refuse to shadow anything on `PATH`, and an isolation test with positive controls. Not yet run on the reference iMac, which is the machine phase 0 shipped to. The two rules it establishes are CLAUDE.md 7a and 7b. |
| [0035](0035-the-desktop-is-a-prompt.md) | The desktop is a prompt — a floating dock-prompt, no top bar | accepted, partially executed | 3 | §4's consent surface shipped: `shell/consent/lisa-consentd.js` and `dev.lisaos.Consent1` split the confirmation UI out of the model host (#135). The rest of the wireframe — §2's prompt in the dock above all — is still design. |
| [0036](0036-an-assistant-that-acts-on-its-own.md) | An assistant that acts on its own — triggers, trust, and what happens when nobody is watching | accepted, partially executed | 4 | corrected 2026-08-06 from "proposed — design only, no code". §6 shipped: `ShellTool` (`libs/forge-harness/src/shell_tool.rs`) is the one shell tool for the long tail, and all four of its conditions are code — jailed and Landlock-confined (#307), guard-checked through `lisa_guard`, never Silent, and never unattended (the consent callback is the only constructor). §1–§5 — trigger classes, the unattended ceiling, standing grants and the "while you were away" review surface — are still design, and the triggers this ADR is named for do not exist. |
| [0037](0037-the-browser-is-a-lisa-app.md) | Browser — the web becomes an agent surface, not a vendored binary | accepted, partially executed | 3 | Surfer ships in the image with tabs, extract and read-tier tools, verified on the device 2026-08-02; write-tier navigate/click/fill landed 2026-08-03 (#166). Device acceptance of the write path is the open remainder. |
| [0038](0038-lisa-desktop-a-hard-fork-of-gnome-shell.md) | Lisa Desktop — a hard fork of GNOME Shell | accepted, partially executed | 3 | and widened by ADR-0048 from the Shell to the whole desktop experience. Step 1 (design tokens + the `check-tokens.py` gate) shipped 2026-08-03. Step 2 lives on `lisa-desktop`'s `vendor-gnome-shell-50.3` branch, not in this repo: the fork builds from a hash-pinned 50.3 tarball, `provides=`/`conflicts=` stock gnome-shell rather than depending on it, and boots headless owning `org.gnome.Shell` — with a deliberately EMPTY Lisa delta, because the milestone is "can we own this". Nobody has logged into it (lisa-desktop#1). `shell/desktop` here is still the extension of the extension era, which step 3 absorbs. |
| [0039](0039-the-split-and-the-package-index.md) | The split, and the package index that makes it work | accepted, partially executed | 3 | executed through the index going live: repos extracted with history, per-repo packages built by CI, `[lisa]` hosted, signed, and pacman-verified from a clean machine (lisa-os#171). **Step 4 is wired**: `os/mkosi/mkosi.pkgmngr/etc/pacman.d/lisa.conf` configures `[lisa]` for the image build and `mkosi.conf` installs `lisa-desktop-shell` from it by name, in the line stock `gnome-shell` used to occupy. Lisa Desktop is step 4's first consumer, and the only one so far — every other Lisa package still arrives through release.yml's locally built `PackageDirectories=`, which keeps precedence over the index. **Step 5 is started, not finished**: the release job now asserts against the mounted image that the shell is ours, that stock gnome-shell is absent, that the session is present and default, and that the extensions, schemas, dconf defaults and app entries are at paths something reads. What it does not assert — and cannot from CI — is that a human logs in. Step 6 (removal from the monorepo) is untouched. |
| [0040](0040-docs-live-with-the-code-no-docs-repo.md) | Docs live with the code — there is no docs repo | accepted | 2 | docs live with the code; `os/repo-tools/build-knowledge.py` is the one curation step with two consumers (the on-device pack and the lisaos.dev build), gated by `just lint`. |
| [0041](0041-package-signing-and-the-trust-chain.md) | Package signing and the trust chain | accepted | 3 | the `[lisa]` index has published signed since 2026-08-03, with `lisa-keyring` shipping the pinned key. SigLevel flips from Optional to Required one release after devices take the keyring. |
| [0042](0042-field-device-keyring-policy.md) | The field device runs a blank login keyring | accepted, partially executed | 1 | decided 2026-08-03; the change awaits one human visit to the reference iMac, because the keyring password is not remotely known. |
| [0043](0043-the-model-knows-the-os-through-retrieval.md) | The model knows the OS through retrieval, never through the prompt | accepted, partially executed | 3 | phase 1 shipped (#175: the pack, the generator, `system` provenance, `lisa context sync-knowledge`, the session-start unit), and answers were verified semantically on the device. Open: retrieval wiring in the assistant and overlay lanes, `--help` in the pack, and the on-device answer-quality eval. |
| [0044](0044-retrieval-receipts.md) | Retrieval receipts — contextd vouches for what it returned | proposed | — | design only; the full design with sequencing is on #55. This file records the decision-shape so it survives sessions. |
| [0045](0045-calver-for-the-image-semver-for-the-contracts.md) | CalVer for the image, SemVer for the contracts | accepted | 1 | both schemes were already in use; this ADR names them and retires the ordinal shorthand that caused the confusion. |
| [0046](0046-capability-before-storefront.md) | Capability before storefront: what must be true before Lisa distributes somebody else's app | accepted | 2 | in force by construction: Lisa distributes nobody else's app, and no storefront exists. Amendment 1 ("source in, source out") is the standing rule for what may ever be distributed; the capability gates it names are tracked by ADR-0049 and #240. |
| [0047](0047-one-toolkit-gjs-gtk4.md) | One toolkit: GJS + GTK4/Adwaita is the default, Flutter is parked | accepted | 4 | GJS + GTK4/Adwaita is the documented default, #37 is closed won't-do, and PLAN §5.8/§5.12 and ADR-0004 carry the correction. Not yet done: `lisa_ui` becoming the shared GJS library — the MCP edge it is meant to de-duplicate still exists in triplicate (`apps/mail`, `apps/preview` and `apps/surfer` each carry their own `lib/mcp.js` and `lib/mcp-protocol.js`). Corrected 2026-08-06: §2 below parks `libs/lisa_ui` and `libs/lisa_flutter` "in the tree" and they were deleted on 2026-08-06 (`d1bdc18`); the name is reserved and the directory does not exist. ADR-0056 is what it becomes. |
| [0048](0048-lisa-desktop-is-a-desktop-not-a-patched-gnome.md) | Lisa Desktop is a desktop, not a patched GNOME | accepted, partially executed | 3 | the core-versus-store test is recorded and PLAN §5.8 is rewritten around "we write the apps"; `gnome-control-center-lisa` is on a retirement path with nothing removed; GTK4/libadwaita and Mutter stay upstream, indefinitely. The desktop half is ADR-0038 step 2 (see there): it builds and boots headless, and **nobody has logged into a session running it**. Of the named core apps, Files and Photos are a README each. |
| [0049](0049-every-app-is-an-agent-surface.md) | Every app is an agent surface: install is the grant, the tier is the gate, the registry is the authority | accepted, not implemented | 3 | the decision stands and the mechanism is largely unbuilt. What exists is the table in §"What exists today" (manifests, tiers at the bus, `lisa tools`, the grant log). Not built: registration at install and deregistration at uninstall, the registry as a stateful authority rather than a startup scan, per-app skills, and stored grant state (#240). |
| [0050](0050-app-tooling-is-cli-and-the-scaffold-carries-the-traps.md) | App tooling is CLI verbs, and the scaffold carries the traps | accepted, partially executed | 3 | the checker exists and the scaffold does not. `lisa dev check` is built (`cli/lisa/src/dev.rs`) and is the Forge's default verifier (`Verifier::Command { program: <current_exe>, args: ["dev", "check"] }` — the running binary rather than the string `"lisa"`, so the verifier cannot pick up a different `lisa` from `$PATH`; corrected 2026-08-06, `cli/lisa/src/main.rs` `default_verifier`), which is how #243 closed; `lisa dev doctor` is built beside it, and `lisa dev` also carries ADR-0034's rootless dev box (`install`/`remove`/`list`/`shell`/`reset`, `cli/lisa/src/devbox.rs`). **`lisa dev new` — the scaffold this ADR is named for — does not exist**, and neither does `lisa dev package`. The previous status line here read "no code exists ... no `lisa dev check`", which was true the day it was written and stale by the next one; a stale status in the *pessimistic* direction is as dangerous as the optimistic kind, because it invites deleting true documentation to match it. |
| [0051](0051-ports-are-built-on-change-not-per-release.md) | Third-party packages are built on change and consumed by pin, not rebuilt per release | accepted, partially executed | 3 | phases 1 and 2 are done. Corrected 2026-08-06: §"Status of execution" below lists the release.yml switch from building to fetching as "not yet done", and it landed in `1acbf42` — `release.yml` now pulls every port from the rolling `ports` release and `sha256sum -c`s it against `os/packages/ports.lock`, refusing a mismatch. Still open: `lisa-desktop-online-accounts` is absent from the lock because its build refuses the placeholder client secret (#276). |
| [0052](0052-install-mode-is-an-image-lineage.md) | Install mode (server/desktop) is an image lineage chosen at install, not a package toggle | superseded in part by ADR-0053 | — | ADR-0053 — the *mechanics* below stand (mode is a lineage, the update channel is part of the mode, never a package toggle), but the framing does not: a few hours after this was written the owner named Lisa Server as a **product** with its own surfaces, not a flavor of the desktop image. ADR-0053 carries the product decision and sequences the lineage below to the day Lisa Server earns its own download page; until then server mode is a boot profile on the one image. |
| [0053](0053-lisa-server-is-a-product-on-the-shared-core.md) | Lisa Server is a product on the shared core, and its first surface is the Assistant as an API | proposed | — | design only, no code. Supersedes ADR-0052's framing of server mode as a flavor; ADR-0052's lineage mechanics remain correct for the day Lisa Server earns its own image. |
| [0054](0054-the-websites-are-generated-not-authored-twice.md) | The websites are generated from the repo, not authored twice | accepted, partially executed | 3 | phase 0 landed: both sites now derive their colours from `branding/tokens.json`, `check-tokens.py` covers `web` as a surface, and `.github/workflows/web.yml` builds and link-checks both sites on PR. Phases 1–3 (Nuxt UI primitives, `@nuxt/content` over `docs/*.md`, derived news/downloads/API reference) have not landed and are tracked as one issue. |
| [0055](0055-the-live-usb-is-the-image-on-removable-media.md) | The live USB is the one image on removable media; liveness is where it booted from, not a lineage | accepted, partially executed | 3 | the medium and the boot are what ship today and are CI-gated; the *guarantee* below (a live session touches only the disk it booted from) is enforced in the installer as of this ADR and only mitigated, never verified, in the mount path. §"What is not built" is the honest list. |
| [0056](0056-lisa-ui-is-the-dialect-not-the-toolkit.md) | `lisa_ui` is the dialect, not the toolkit | accepted, partially executed | 2 | **step 1 landed 2026-08-06**: the Agent Bus edge is one file at `apps/lisa_ui/mcp/protocol.js` and Mail, Surfer and Preview import it. Steps 2–4 (token sheet loading, `LisaWindow`, widgets) are unbuilt, so #282 is not closed. |
| [0057](0057-the-monorepo-owns-the-surfaces-until-step-6.md) | the monorepo owns the shell surfaces until step 6 actually happens | accepted | 3 | — |
| [0058](0058-the-desktop-inventory-owned-foundation-interim.md) | the desktop inventory: owned, foundation, interim | accepted | 9 | — |
| [0059](0059-remoted-brokers-model-egress-not-every-socket.md) | `lisa-remoted` brokers model egress, not every socket on the machine | accepted, partially executed | 3 | the reasoning and the exemption are recorded here and cited from `os/repo-tools/check-egress-units.py`. The remaining edit is CLAUDE.md rule 5's own wording, quoted verbatim in "The wording rule 5 should carry" below; until that lands, the operating manual still states the absolute this record retires. |
| [0060](0060-the-app-bundle-lisa-framework-lisa-sdk.md) | the app bundle, `lisa.framework` and `lisa.sdk` | accepted, not implemented | 4 | this record fixes the shape before the code exists, because the alternative is seven more surfaces hand-rolling the same proxies while the shape stays folklore. |
| [0061](0061-lisa-coder-grows-from-forge-harness.md) | Lisa Coder grows from forge-harness; other harnesses are quarries | accepted | 2 | — |
| [0062](0062-one-summon-surface.md) | one summon surface: the typed ask lives in Spotlight | accepted | 2 | — |

<!-- END GENERATED INDEX -->

## Process

1. Copy the template below to `NNNN-short-slug.md` (next free number).
2. Give it a status line in the canonical shape — the generator rejects
   anything else, and rejects a state outside the vocabulary:
   `proposed`, `accepted`, `accepted, partially executed`,
   `accepted, not implemented`, `superseded by ADR-NNNN`,
   `superseded in part by ADR-NNNN`, `status unverified`.
   Name the open steps; "largely done" is not a status.
3. When the state changes, edit the status line and re-run
   `python3 os/repo-tools/build-adr-index.py`. Do not rewrite the
   argument — a decision that turned out wrong is more useful with its
   original reasoning intact and a supersession marker on top.
4. Reference the ADR from commits and the affected component README.

## Template

```markdown
# ADR-NNNN — Title

- **Status:** accepted — one clause on where this actually stands
- **Date:** YYYY-MM-DD

## Context
What forced a decision.

## Decision
What we chose, stated imperatively.

## Consequences
What gets easier, what gets harder, what we gave up.
```
