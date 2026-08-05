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

Read it as: **what would a person find on a device today.** Entries are
expected to name their evidence — an earlier table listed ADR-0025 among
the unbuilt because someone grepped for `Agent1` and found tooltips
instead of following `RunSync` to `dev.lisaos.Harness1`. A table of what
is built is worth less than nothing if its entries are inferred from a
grep.

<!-- BEGIN GENERATED INDEX — os/repo-tools/build-adr-index.py; edit the ADRs, not this table -->

**54 records** — 5 superseded in part, 22 accepted and partly executed, 2 accepted with no code yet, 21 accepted and done, 4 proposed.

| ADR | Decision | Status | Where it actually stands |
|---|---|---|---|
| [0001](0001-arch-immutable-base.md) | Fork Arch Linux; ship an immutable, atomic, image-based OS via mkosi | accepted | the mkosi/UKI A/B image builds, boots, and demonstrates update *and* rollback in CI. |
| [0002](0002-rust-zbus-axum.md) | Rust with zbus + axum for system daemons | accepted | every daemon under `daemons/` is Rust on zbus, with axum where there is HTTP. |
| [0003](0003-two-track-delivery.md) | Two-track delivery — Lisa Layer first, immutable image as the product | accepted | both tracks ship: Track L installs onto stock Arch from the signed `[lisa]` index, Track I is the released image. |
| [0004](0004-flutter-lane-forge.md) | Flutter app lane + the Forge | superseded in part by ADR-0047 | the lane split is no longer in force: GJS + GTK4/Adwaita is the default for Lisa's apps and for Forge output, and Flutter is parked. The Forge itself (PLAN §5.12.1) stands, and the spike findings at the foot of this file stand as history. |
| [0005](0005-gpl2-license.md) | License the project GPL-2.0-only | accepted | — |
| [0006](0006-monorepo-staged-extraction.md) | Monorepo with staged extraction | accepted | extended, not superseded, by ADR-0039: none of this ADR's own four triggers has fired; the two that fired are ones it could not have contained. |
| [0007](0007-fcitx5-addon-cxx.md) | fcitx5-lisa is a C++ addon (thin), logic stays on the daemon side | accepted | the addon builds against fcitx5 in CI and its protocol logic is unit-tested (`just ime-test`). |
| [0008](0008-portal-standalone-service.md) | The Lisa portal is a standalone session service, consent stays in the shell | accepted | installed on the device since v20260730.55 (#153). |
| [0009](0009-agent-bus-core.md) | Agent Bus core — D-Bus surface, tier enforcement at the bus, staged MCP transport | accepted, partially executed | the bus, the tier state machine, provenance escalation and the undo journal are live in `daemons/agentd`, and MCP genuinely rides per-app unix sockets (`libs/mcp-bus`, `McpDispatcher`), not in-process dispatch. Open: socket activation (`mcp.activatable` is declared and unimplemented), and the §5.4 acceptance flow end to end. |
| [0010](0010-remote-providers.md) | BYO remote model providers via a dedicated egress broker (`lisa-remoted`) | accepted | `lisa-remoted` is the sole egress broker for provider traffic; PKCE state fixed (#110). A live sign-in on the device is still the outstanding confirmation. |
| [0011](0011-ambient-assistant.md) | Lisa Ambient — the always-on, wake-word-free assistant | accepted, partially executed | corrected 2026-08-04, because "NO implementation" was read off the wrong subject. The primitives exist and are proven on the reference iMac (2026-07-31): `lisa listen`/`transcribe` on packaged whisper.cpp, `lisa say` on packaged piper, `lisa ambient classify`, push-to-talk over `dev.lisaos.Voice1`, both engines installed by the image lane and both voice models pinned in the catalog. What does not exist is this ADR's actual subject — the always-on loop (VAD, ring buffer, hard mute, addressed-intent classification running unprompted) and the `voiced` daemon. Nothing in the repo records unprompted (#158). |
| [0012](0012-gnome-control-center-lisa-panel.md) | A native "Intelligence" panel in a forked gnome-control-center | accepted | the panel ships in the image and runs on the device (v25+), including provider OAuth. ADR-0048 §3 puts `gnome-control-center-lisa` on a path to retirement in favour of `shell/settings`; nothing is removed yet. |
| [0013](0013-harness-intents-and-coding-agent.md) | The Lisa harness — Siri-style intents + a Claude-Code-level coding agent, on the existing substrate | accepted, partially executed | Sessions, Skills, Memory and the policy layer shipped in `libs/harness-core` and `lisa forge` runs on them. Remaining pillar: Crons, deliberately last (ADR-0025 phase 5). |
| [0014](0014-lisa-ui-material-fork.md) | lisa_ui becomes the kit Lisa apps import — Material-backed now, vendored fork later | superseded in part by ADR-0047 | `lisa_ui` keeps the name and the role, but it is now the shared **GJS/GTK4** library rather than a Material-backed Flutter kit, and the vendored-fork endgame is parked with the lane. The argument for owning the kit stands; the toolkit it named does not. |
| [0015](0015-assistant-app.md) | a persistent Assistant chat window — the surface that makes the model usable | accepted, partially executed | the window ships, streams and is ledgered on the device, and read-tier tools reach it through `dev.lisaos.Harness1`. No write tier and no memory across conversations (#157). **Widened by ADR-0053:** this ADR's "one headless backend, many thin frontends" is the pattern Lisa Server extends over a network — the GJS window becomes one frontend among several (web, API, mobile) rather than *the* Assistant, which makes the backend contract public API and something to version deliberately. |
| [0016](0016-reverse-dns-naming.md) | reverse-DNS identifiers move to the real domains (dev.lisaos.* / app.lisaos.*) | accepted | — |
| [0017](0017-plymouth-in-initrd.md) | Plymouth + the lisa theme move into the mkosi-initrd | accepted, partially executed | the `simpledrm`-only display clause is amended by ADR-0026 and the delivery mechanism by ADR-0028. Plymouth is genuinely in the initrd and asserted in the nightly; the splash→desktop handoff has never been seen on hardware (#26). |
| [0018](0018-var-pinned-partuuid.md) | /var is mounted by partition LABEL, not by UUID | accepted | — |
| [0019](0019-dedicated-home-partition.md) | a dedicated /home partition on fresh installs, weight-split with var | accepted | — |
| [0020](0020-app-update-channel.md) | app updates decoupled from the OS image | accepted | `lisa apps update/rollback/sync` ship and devices pull payloads. The channel is monolithic, which per-app store versioning will have to change (#239). |
| [0021](0021-aarch64-lane.md) | aarch64 image lane on an Arch Linux ARM base | accepted, partially executed | the aarch64 image builds and boots in CI on an ALARM base with the same package set as x86_64. No aarch64 image has been published, and ARM has speech in but not out (no onnxruntime on Arch Linux ARM). |
| [0022](0022-rescue-boot-path.md) | A user-survivable rescue boot path | accepted, partially executed | phases 1–3 are implemented and proven by execution (phase 1 self-repair and phase 3's shell on an unbootable machine are green in ab-recovery; phase 2's resolver refuses a half-written slot on its GPT type in ab-interrupted-transfer). Still missing: the boot ENTRY — the resolver works, nothing offers it in the menu (#23). |
| [0023](0023-slim-core-var-grows.md) | Slim core, /var grows — apps and heavy payloads leave the image | accepted, partially executed | phase 1 complete and device-verified (the baked `/opt/zen` left both image lanes, #89). Phase 2 (installer pre-pull) not started. Phase 3 (slot shrink) was tried at 7G and reverted the same day: the `du` figure this ADR reasoned from is not the quantity that governs slot size — see "Phase 3, attempted". |
| [0024](0024-apple-cs8409-out-of-tree-codec.md) | ship an out-of-tree CS8409 codec module for Apple speakers | accepted, partially executed | the module is packaged and re-pinned at every kernel bump, and a mismatched pin fails the build loudly by design. The reference iMac's speakers have still never made a sound through it (#44). |
| [0025](0025-one-agent-loop.md) | One agent loop — the Lisa harness | accepted, partially executed | one loop exists: the Assistant runs on `dev.lisaos.Harness1` and reaches the Agent Bus through `bus-tools`. Skills carry an enforced tool allowlist that no shipped skill populates (#245); phase 5 (Crons) is not started. |
| [0026](0026-native-drm-in-initrd.md) | The native GPU driver + its firmware ride the initrd | accepted | — |
| [0027](0027-flutter-on-device-aarch64-and-forged-app-launch.md) | the Flutter lane on-device — aarch64 SDK, and how a forged app gets launched | superseded in part by ADR-0047 | #37 is closed won't-do, so the on-device Flutter SDK, the aarch64 pin and the forged-app build/launch path are parked with the lane. §3 (Skills live in `skills/<name>/SKILL.md`, installed to `/usr/share/lisa/skills`) is unaffected and in force. |
| [0028](0028-initrd-overlay-mechanism.md) | Files reach the default initrd through `io.mkosi.initrd`, not `mkosi.initrd/` | accepted | — |
| [0029](0029-hard-guardrails-for-agent-actions.md) | Hard guardrails for agent actions — policy outside the model | accepted, partially executed | phases 1–3 implemented, with three adversarial review rounds folded in (corpus 49 → 128 denied). Open: Landlock confinement of forge subprocesses (#53), and `lisa suggest` still emits a shell string rather than the structured argv this ADR's own post-mortem calls for (#88). |
| [0030](0030-the-guardrail-boundary.md) | The guardrail boundary — probabilistic inside, logical outside | accepted | the principle has teeth rather than prose: #145 and #55 were both closed against it, and `lisa guard list\|allow\|forbid` is the owner's out-of-band relaxation, where no tool call can reach it. |
| [0031](0031-server-mode-two-edges-and-artifact-publishing.md) | Server mode, the two edges, and artifact publishing | superseded in part by ADR-0053 | still proposed, still no code (no `serverd`, no `lisa serve`, neither edge exists). ADR-0053 promotes §1's "server mode is a flavor chosen at install" into a **product** with its own surfaces and a sequencing ladder, and re-decides §2's management/use split: the first server surface is the Assistant as an API/web frontend of the existing backend, not a Cockpit module. §3 (the two network edges) and §4 (artifact publishing) stand, and are what ADR-0053's network-identity work builds on. |
| [0032](0032-construct-and-lisa-one-contract-two-levels.md) | Construct and Lisa — one contract, two levels | proposed | design only, no code. The shared contract (manifest, provenance vocabulary, Ledger event shape, tokens) is defined on the Lisa side only. |
| [0033](0033-identity-comes-from-the-transport.md) | Identity comes from the transport, not the message | accepted, partially executed | `libs/lisa-peer` is the primitive, and agentd, contextd, harnessd, remoted and the portal all link it. The sweep for the remaining callers is unfinished; the rule itself is CLAUDE.md 6b. |
| [0034](0034-lisa-dev-user-scope-tooling.md) | `lisa dev` — developer tooling in the user's home, rootless | accepted, partially executed | phases 0, 1 and 2 are implemented and proven by execution on a real rootless podman: `lisa dev install\|remove\|list\|shell\|reset\|doctor\|check`, a /home disk guard that measures the container store's own filesystem, shims that refuse to shadow anything on `PATH`, and an isolation test with positive controls. Not yet run on the reference iMac, which is the machine phase 0 shipped to. The two rules it establishes are CLAUDE.md 7a and 7b. |
| [0035](0035-the-desktop-is-a-prompt.md) | The desktop is a prompt — a floating dock-prompt, no top bar | accepted, partially executed | §4's consent surface shipped: `shell/consent/lisa-consentd.js` and `dev.lisaos.Consent1` split the confirmation UI out of the model host (#135). The rest of the wireframe — §2's prompt in the dock above all — is still design. |
| [0036](0036-an-assistant-that-acts-on-its-own.md) | An assistant that acts on its own — triggers, trust, and what happens when nobody is watching | proposed | design only, no code; it depends on ADR-0025's loop, which exists, and on triggers, which do not. |
| [0037](0037-the-browser-is-a-lisa-app.md) | Browser — the web becomes an agent surface, not a vendored binary | accepted, partially executed | Surfer ships in the image with tabs, extract and read-tier tools, verified on the device 2026-08-02; write-tier navigate/click/fill landed 2026-08-03 (#166). Device acceptance of the write path is the open remainder. |
| [0038](0038-lisa-desktop-a-hard-fork-of-gnome-shell.md) | Lisa Desktop — a hard fork of GNOME Shell | accepted, partially executed | and widened by ADR-0048 from the Shell to the whole desktop experience. Step 1 (design tokens + the `check-tokens.py` gate) shipped 2026-08-03. Step 2 lives on `lisa-desktop`'s `vendor-gnome-shell-50.3` branch, not in this repo: the fork builds from a hash-pinned 50.3 tarball, `provides=`/`conflicts=` stock gnome-shell rather than depending on it, and boots headless owning `org.gnome.Shell` — with a deliberately EMPTY Lisa delta, because the milestone is "can we own this". Nobody has logged into it (lisa-desktop#1). `shell/desktop` here is still the extension of the extension era, which step 3 absorbs. |
| [0039](0039-the-split-and-the-package-index.md) | The split, and the package index that makes it work | accepted, partially executed | executed through the index going live: repos extracted with history, per-repo packages built by CI, `[lisa]` hosted, signed, and pacman-verified from a clean machine (lisa-os#171). **Step 4 is wired**: `os/mkosi/mkosi.pkgmngr/etc/pacman.d/lisa.conf` configures `[lisa]` for the image build and `mkosi.conf` installs `lisa-desktop-shell` from it by name, in the line stock `gnome-shell` used to occupy. Lisa Desktop is step 4's first consumer, and the only one so far — every other Lisa package still arrives through release.yml's locally built `PackageDirectories=`, which keeps precedence over the index. **Step 5 is started, not finished**: the release job now asserts against the mounted image that the shell is ours, that stock gnome-shell is absent, that the session is present and default, and that the extensions, schemas, dconf defaults and app entries are at paths something reads. What it does not assert — and cannot from CI — is that a human logs in. Step 6 (removal from the monorepo) is untouched. |
| [0040](0040-docs-live-with-the-code-no-docs-repo.md) | Docs live with the code — there is no docs repo | accepted | docs live with the code; `os/repo-tools/build-knowledge.py` is the one curation step with two consumers (the on-device pack and the lisaos.dev build), gated by `just lint`. |
| [0041](0041-package-signing-and-the-trust-chain.md) | Package signing and the trust chain | accepted | the `[lisa]` index has published signed since 2026-08-03, with `lisa-keyring` shipping the pinned key. SigLevel flips from Optional to Required one release after devices take the keyring. |
| [0042](0042-field-device-keyring-policy.md) | The field device runs a blank login keyring | accepted, partially executed | decided 2026-08-03; the change awaits one human visit to the reference iMac, because the keyring password is not remotely known. |
| [0043](0043-the-model-knows-the-os-through-retrieval.md) | The model knows the OS through retrieval, never through the prompt | accepted, partially executed | phase 1 shipped (#175: the pack, the generator, `system` provenance, `lisa context sync-knowledge`, the session-start unit), and answers were verified semantically on the device. Open: retrieval wiring in the assistant and overlay lanes, `--help` in the pack, and the on-device answer-quality eval. |
| [0044](0044-retrieval-receipts.md) | Retrieval receipts — contextd vouches for what it returned | proposed | design only; the full design with sequencing is on #55. This file records the decision-shape so it survives sessions. |
| [0045](0045-calver-for-the-image-semver-for-the-contracts.md) | CalVer for the image, SemVer for the contracts | accepted | both schemes were already in use; this ADR names them and retires the ordinal shorthand that caused the confusion. |
| [0046](0046-capability-before-storefront.md) | Capability before storefront: what must be true before Lisa distributes somebody else's app | accepted | in force by construction: Lisa distributes nobody else's app, and no storefront exists. Amendment 1 ("source in, source out") is the standing rule for what may ever be distributed; the capability gates it names are tracked by ADR-0049 and #240. |
| [0047](0047-one-toolkit-gjs-gtk4.md) | One toolkit: GJS + GTK4/Adwaita is the default, Flutter is parked | accepted | GJS + GTK4/Adwaita is the documented default, #37 is closed won't-do, and PLAN §5.8/§5.12 and ADR-0004 carry the correction. Not yet done: `libs/lisa_ui` becoming the shared GJS library — the MCP edge it is meant to de-duplicate still exists in triplicate. |
| [0048](0048-lisa-desktop-is-a-desktop-not-a-patched-gnome.md) | Lisa Desktop is a desktop, not a patched GNOME | accepted, partially executed | the core-versus-store test is recorded and PLAN §5.8 is rewritten around "we write the apps"; `gnome-control-center-lisa` is on a retirement path with nothing removed; GTK4/libadwaita and Mutter stay upstream, indefinitely. The desktop half is ADR-0038 step 2 (see there): it builds and boots headless, and **nobody has logged into a session running it**. Of the named core apps, Files and Photos are a README each. |
| [0049](0049-every-app-is-an-agent-surface.md) | Every app is an agent surface: install is the grant, the tier is the gate, the registry is the authority | accepted, not implemented | the decision stands and the mechanism is largely unbuilt. What exists is the table in §"What exists today" (manifests, tiers at the bus, `lisa tools`, the grant log). Not built: registration at install and deregistration at uninstall, the registry as a stateful authority rather than a startup scan, per-app skills, and stored grant state (#240). |
| [0050](0050-app-tooling-is-cli-and-the-scaffold-carries-the-traps.md) | App tooling is CLI verbs, and the scaffold carries the traps | accepted, partially executed | the checker exists and the scaffold does not. `lisa dev check` is built (`cli/lisa/src/dev.rs`) and is the Forge's default verifier (`Verifier::Command { program: "lisa", args: ["dev", "check"] }`, `cli/lisa/src/main.rs` `default_verifier`), which is how #243 closed; `lisa dev doctor` is built beside it, and `lisa dev` also carries ADR-0034's rootless dev box (`install`/`remove`/`list`/`shell`/`reset`, `cli/lisa/src/devbox.rs`). **`lisa dev new` — the scaffold this ADR is named for — does not exist**, and neither does `lisa dev package`. The previous status line here read "no code exists ... no `lisa dev check`", which was true the day it was written and stale by the next one; a stale status in the *pessimistic* direction is as dangerous as the optimistic kind, because it invites deleting true documentation to match it. |
| [0051](0051-ports-are-built-on-change-not-per-release.md) | Third-party packages are built on change and consumed by pin, not rebuilt per release | accepted, partially executed | — |
| [0052](0052-install-mode-is-an-image-lineage.md) | Install mode (server/desktop) is an image lineage chosen at install, not a package toggle | superseded in part by ADR-0053 | the *mechanics* below stand (mode is a lineage, the update channel is part of the mode, never a package toggle), but the framing does not: a few hours after this was written the owner named Lisa Server as a **product** with its own surfaces, not a flavor of the desktop image. ADR-0053 carries the product decision and sequences the lineage below to the day Lisa Server earns its own download page; until then server mode is a boot profile on the one image. |
| [0053](0053-lisa-server-is-a-product-on-the-shared-core.md) | Lisa Server is a product on the shared core, and its first surface is the Assistant as an API | proposed | design only, no code. Supersedes ADR-0052's framing of server mode as a flavor; ADR-0052's lineage mechanics remain correct for the day Lisa Server earns its own image. |
| [0054](0054-the-websites-are-generated-not-authored-twice.md) | The websites are generated from the repo, not authored twice | accepted, not implemented | the direction is decided; the phases below are tracked as one issue and none has landed. |

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
