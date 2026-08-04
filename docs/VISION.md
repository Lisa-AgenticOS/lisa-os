# Lisa OS — the one page

**Read this first.** `docs/PLAN.md` is the architecture and scope,
`docs/adr/` is the reasoning, `docs/STATUS.md` is the running log. This
page is what a person or a session needs before any of them: what Lisa
is, what it is made of, which decisions are in force, and — the part
that keeps getting lost — **what is actually true today versus what is
only decided**.

Everything below is derived from the tree, not from the plans. Where a
document and the tree disagree, the tree wins and this page says so.

*State of the repo when this was written: `lisa-os` at `6fdc8f7`,
2026-08-04.*

---

## What Lisa is

An **AI-native Linux distribution**: not a desktop with a chatbot in it,
but an OS where intelligence is a system service.

1. **Inference is infrastructure.** Like sound or display, model
   inference is a shared, arbitrated system resource — one daemon owns
   the compute budget, apps get sessions through a portal. No app ships
   its own 5 GB model.
2. **Every app has context.** Each app gets a private, durable,
   user-inspectable memory, plus scoped and consented access to a
   system-wide personal index. Context is a capability you grant, not
   data apps scrape.
3. **Every app is an agent surface.** Apps declare their actions as MCP
   tools. Installing an app is how the machine learns to do a new thing;
   uninstalling it is how it forgets.
4. **Radical legibility.** It runs local by default with egress blocked
   by mechanism rather than by policy, and an append-only **Ledger**
   means **every action it took is in the Ledger, in plain language,
   forever**. If a Golden Gate user asks *"what did Siri actually
   read?"* there is no answer; on Lisa the Ledger **is** the answer.
   Always-on is only acceptable because it is always legible.

The positioning line: *macOS 27 gives you Apple's intelligence. Lisa
gives you yours.* The experiential one: the warmth of *Her*, on hardware
you own, in a log you can read — a companion that learns only what you
give it and **does not get discontinued by a vendor**.

Two further ideas the ADRs lean on and this page will not repeat:
**make and serve** — your hardware builds the artifact *and* serves it,
which nobody else closes (ADR-0031) — and **Construct and Lisa are one
thesis at two levels**, sharing a contract (manifest, provenance
vocabulary, Ledger event shape, design tokens) and not code (ADR-0032).
Both are proposed and unbuilt; see the second list below.

## What it is made of

**Four repos, one product.** The split happened on 2026-08-02 (ADR-0039)
and was a packaging decision, not a rewrite:

| repo | holds |
|---|---|
| `lisa-os` (this one) | daemons, portal, SDK, CLI, guard, the image (`os/mkosi`), the layer (`os/layer`), apps and shell surfaces that have not moved |
| `lisa-desktop` | the shell surfaces, the IME, and the vendored GNOME Shell fork |
| `lisa-apps` | the GJS app set |
| `lisa-packages` | the hosted `[lisa]` pacman index |

**ADR-0006 still governs the monorepo**: split by exception, on named
triggers. None of its own four triggers has fired; the two that did were
created by later decisions it could not have contained. `liblisa` and the
Flutter lane are held on purpose.

**Everything is a package.** Each repo builds its own pacman packages in
CI; `lisa-packages` publishes them into a **signed `[lisa]` index**
(ADR-0041), trust anchored by `lisa-keyring` with a pinned fingerprint.

**Two delivery tracks** (ADR-0003). **Track L** — `os/layer/install.sh`
onto stock Arch, pulling the hosted signed index, proven zero-config from
a clean container. **Track I** — the immutable mkosi/UKI image with A/B
root slots, `systemd-sysupdate`, boot-counting rollback, `/var` and
`/home` on their own partitions (ADR-0018, ADR-0019). Track L is the
distribution channel while Track I matures; Track I is where the egress
and Ledger guarantees are fully enforceable.

**Versions** (ADR-0045): CalVer for the image, SemVer for the contracts.

## The decisions that are live

- **GJS + GTK4/Adwaita is the one toolkit** for Lisa's apps and Forge
  output; Flutter is parked, not deleted — ADR-0047.
- **Lisa Desktop is its own desktop, not a patched GNOME.** We write the
  apps; we fork the Shell's JavaScript. **GTK4/libadwaita and Mutter stay
  upstream, indefinitely** — toolkit and compositor are foundation, not
  identity — ADR-0038, ADR-0048.
- **Every app is an agent surface.** Install is the grant, the tier is the
  gate, the registry is the authority — ADR-0049.
- **Capability before storefront.** Lisa distributes nobody else's app
  until the capability exists to do it safely; "source in, source out" is
  the standing rule for anything ever distributed — ADR-0046.
- **App tooling is CLI verbs, not an IDE.** Every developer verb lives
  under `lisa dev`, in the user's home, rootless — ADR-0034, ADR-0050.
- **Guardrails are deterministic code the model cannot reach.** Two tests
  before shipping one: *is it reachable from inside?* and *is it aimed at
  the model or at the owner?* Guardrails sit between the model and the
  machine, never between a person and their own machine — ADR-0029,
  ADR-0030, CLAUDE.md 6a.
- **Identity comes from the transport, never from the message.** Ownership
  is the broker-assigned peer name; program identity is peer credentials
  and `/proc/<pid>/exe`, never `comm` — ADR-0033, CLAUDE.md 6b.
- **Egress is architecture.** `inferenced`, `contextd` and `agentd` never
  get network; only `remoted` does — CLAUDE.md rule 5.
- **The install, update and recovery paths depend on nothing we do not
  control**; `/var` is the system's, `$HOME` is the user's — ADR-0034,
  CLAUDE.md 7a/7b.
- **Docs live with the code.** No docs repo; one curation step, two
  consumers — ADR-0040.
- **The model learns the OS by retrieval, never through the prompt** —
  ADR-0043.

## What is true today, and what is only decided

This is the section whose absence caused the drift. Two lists, and
nothing appears in the first without something in the tree behind it.

### True today — verified in the tree at `6fdc8f7`

- **The OS boots, updates itself, and runs on real hardware.** The image
  builds, boots, and demonstrates A/B update *and* rollback in CI
  (`os/mkosi`, `.github/workflows/nightly.yml`); the reference iMac18,2
  runs a released image and pulls updates over the channel.
- **Local inference is a system service.** `lisa-inferenced` supervises
  `llama-server` children, streams tokens, does guided generation
  (JSON-Schema→GBNF, 1000/1000 on the validation gate), embeddings, and
  multi-model residency with LRU eviction. `lisa-modeld` is a blake3
  content-addressed store with a hardware profiler; the catalog pins 17
  artifacts.
- **The Ledger is enforced, not decorative.** Append-only SQLite with
  UPDATE/DELETE aborted by triggers, and it gates inference: no ledger
  entry, no generate.
- **Egress is enforced by unit files, and it is not `PrivateNetwork=`.**
  `lisa-agentd`, `lisa-contextd-user` and `lisa-notes` run
  `IPAddressDeny=any` with `RestrictAddressFamilies=AF_UNIX`;
  `lisa-inferenced` and `lisa-harnessd` are loopback-only;
  **`lisa-remoted` is the single daemon with real egress**, which is its
  whole job.
- **The Agent Bus is real MCP.** Tiers enforced at the bus, provenance
  escalation, an undo journal behind `lisa undo`, and tools reached over
  **per-app unix sockets** (`libs/mcp-bus`), not in-process dispatch.
- **The bus can refuse.** Landed 2026-08-04 (#251/#252):
  `libs/lisa-guard/src/action.rs` judges every tool call, a refused call
  becomes `Outcome::Refused` and is **never parked for a dialog that
  could approve it** (there is a test named exactly that), the consent
  surface renders it, and it lands in the Ledger as `tool.refuse`.
- **The guard has a corpus, not a promise.** ~14 shell rules plus 11 bus
  rules, ~258 corpus cases across seven tables, with a coverage test that
  fails if a hard-no rule has no entry.
- **Identity comes from the transport, in code.** `libs/lisa-peer` is
  linked by agentd, contextd, harnessd, remoted and the portal.
- **The context fabric works, for two sources.** FTS5 + hybrid ranking,
  porter stemming, per-scope ACLs with a 10,000-case fuzz gate that also
  proves itself non-vacuous. Provenances actually written: `file`, `mail`,
  and `system` (the knowledge pack).
- **The portal is the trust boundary**, with per-app identity, first-use
  consent and an append-only grant store — installed on devices since
  v20260730.55.
- **Surfaces people can use:** the Assistant window (streaming,
  ledgered), the overlay, the semantic launcher, the Ledger app, the
  Intelligence panel with provider OAuth, the fcitx5 IME, and the apps
  Mail, Surfer, Preview and the terminal integration. `lisa` has 27
  verbs.
- **Voice v1 exists and was proven on the iMac** (2026-07-31): packaged
  whisper.cpp and piper, `lisa listen`/`say`/`ambient classify`,
  push-to-talk over `dev.lisaos.Voice1`.
- **The `[lisa]` index is live and signed**, and Track L installs from it
  zero-config on a clean machine.

### Decided, and not built

- **The Settings policy page (#253) does not exist.** Refusals happen and
  nothing shows the owner the rules that produced them.
- **Agent scratch does not exist.** `Grant::scratch` is a field that
  nothing populates, so every path is judged as if there were no scratch.
- **The ADR-0049 registry lifecycle is unbuilt (#240).** `Registry` is a
  `BTreeMap` filled by a one-shot directory scan at daemon start. No
  registration at install, no deregistration at uninstall, no stored grant
  state, no per-app skills.
- **The skill allowlist is enforced and never populated (#245).** The
  mechanism is real (`Skill::allowed_by`); no production caller sets it,
  and the tree contains exactly one skill.
- **`lisa dev` does not exist** (ADR-0034 phase 1, ADR-0050). No `dev`
  verb, no scaffold generator, no `lisa dev check` — zero code.
- **There is no storefront.** Zero code, anywhere. Under ADR-0046 that is
  the decision being honoured, not a gap.
- **Lisa Desktop has never been logged into.** On
  `lisa-desktop`'s `vendor-gnome-shell-50.3` branch the fork builds from a
  hash-pinned 50.3 tarball, replaces stock `gnome-shell` via
  `provides=`/`conflicts=`, and boots headless owning `org.gnome.Shell` —
  with a deliberately **empty Lisa delta**, because step 2's milestone is
  "can we own this". Nobody has selected it at a GDM greeter and got a
  desktop (lisa-desktop#1). What ships today is still the extension era.
- **The image does not consume `[lisa]`** — ADR-0039 step 4. Both image
  workflows still build from locally-built packages.
- **Ambient — this project's headline idea — is not built.** The
  primitives run; the always-on loop (VAD, ring buffer, hard mute,
  addressed-intent classification running unprompted) and the `voiced`
  daemon do not exist. Nothing in the repo records unprompted (#158).
- **Server mode / the Personal Compute Node is prose.** No `lisa-node`, no
  `serverd`, no `lisa serve`, no edge (ADR-0031, PLAN §5.11).
- **Files, Photos and Recorder are a README each.** Notes is an MCP server
  with no window yet.
- **Nothing is encrypted at rest, and nothing is sandboxed by Flatpak.**
  LUKS2/TPM2 enrolment, the encrypted context index, and Flatpak-first app
  delivery are all plan, not code — apps are unsandboxed GJS launched from
  a pacman payload.
- **liblisa has no C ABI, no GObject Introspection and no bindings**;
  `liblisa-gtk` and `liblisa-qt` are placeholder READMEs. Writing Tools
  layer 2 (the IME) is real; layers 1 and 3 do not exist. §5.7.4 screen
  context does not exist — what exists is user-pasted images.
- **No LoRA adapters are trained or pinned**, and Track L has no Snapper
  rollback.

## What is next

Short, and only what is in flight.

1. **Get a human logged into Lisa Desktop.** The fork builds and boots
   headless; the whole question is the session — GDM's list,
   `gnome-session --session=lisa`, and the dock/launcher defects found the
   moment anyone looks (#255, #262, #263, #266, #267).
2. **Close ADR-0039 step 4** — the image consumes `[lisa]` instead of a
   build directory. It is the last step that makes the split real rather
   than parallel.
3. **Give the refusal a face and the registry a life** — the Settings
   policy page (#253) and the ADR-0049 install/uninstall lifecycle (#240),
   which between them turn "the bus can refuse" into something an owner
   can see and steer.

Everything else is backlog until one of these is done. A milestone is
done when its Acceptance block passes in CI; a feature is real when
someone has watched it work on the iMac.
