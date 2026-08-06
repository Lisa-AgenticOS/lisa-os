# ADR-0046 — Capability before storefront: what must be true before Lisa distributes somebody else's app

- **Status:** accepted — in force by construction: Lisa distributes nobody
  else's app, and no storefront exists. Amendment 1 ("source in, source
  out") is the standing rule for what may ever be distributed; the
  capability gates it names are tracked by ADR-0049 and #240.
- **Date:** 2026-08-04
- **Supersedes:** nothing. Extends ADR-0020 (apps channel), ADR-0039 (repo
  split and `[lisa]`), ADR-0029/0030 (guardrails), ADR-0033 (identity from
  the transport).
- **Anticipated by:** ADR-0023, which names third-party app distribution as
  a *future* goal rather than a current one.
- **Claims:**
  - `symbol:fn apply_tier_floor@daemons/agentd/src/manifest.rs` — the capability this ADR says must come before a storefront
  - `path:daemons/agentd/src/registry.rs` — the registry it is enforced from

## Context

"Let's build an app store" is one sentence describing two projects with
different risk profiles, and conflating them is how the dangerous one gets
built by accident.

The substrate for distribution already exists. ADR-0020 decoupled the apps
channel from the image, so payloads land in `/var` and `lisa apps update`
refreshes them without a new image. ADR-0039 split the repos and got
`[lisa]` hosted, with v0.1.0 published and verified through pacman.
`lisa-keyring` ships the signing key in the image. Every Lisa app already
carries a manifest declaring its agent surface. A window over that is
weeks of UI, not a research project.

The reason to stop and write this down is what a storefront *asserts*. A
list of installable things makes a claim — *this is safe to install* — and
on 2026-08-04 an adversarial review of the three apps we wrote ourselves
returned 27 verified defects. Among them: agent tool scripts running in
the page's own JavaScript world, so a hostile page could forge everything
the model read and redirect a `fill` into a password field (#212); the
agent boundary accepting `file://`, so `/etc/passwd` entered the model's
context tagged `provenance:"web"` (#214); a tool dispatcher resolving
through `Object.prototype` and answering **success** for `constructor`
(#218); and 98 MB of attachment bytes flowing into the model as message
bodies across 168 messages (#221).

Every one of those was in code we wrote, reviewed, and shipped. None was
found by the test suites, because the fixtures made the guards no-ops.
Third-party code is the same problem without the part where we can fix it.

There is also a design reason, and it is the more interesting one. A Lisa
storefront's distinctive value is not listing packages — GNOME Software
exists and works. It is that **every app is an agent surface** (PLAN §29).
The screen worth building shows what an app can do *to your machine and on
your behalf*: which tools, at which tiers, what egress, what provenance it
may act on. No other desktop can render that page, because no other
desktop has the manifest.

That page is only worth rendering if it is true.

## The problem, stated precisely

Today a manifest **asserts** `tier: read | write | destructive`. What
actually holds the line is `read_tier_tools` in `libs/bus-tools`, which
filters the agent loop to read-tier tools regardless of what a manifest
claims. That filter is the mechanism; the manifest is the label.

For apps we build and sign, an asserted tier is fine — the assertion and
the enforcement have the same author, and a wrong label is a bug we own.
For a stranger's app, an asserted tier is a *suggestion from the party
with the most to gain by lying*. ADR-0029's first test applies directly:
a check the untrusted party can influence is not a guardrail.

So the gap is not the storefront. The gap is that capability is
**declared** where it needs to be **verified**.

## Decision

Third-party app distribution is gated on capability enforcement. We build
it in this order, and we do not skip a step because the UI is ready.

### 1. First-party catalog — unblocked, build whenever

A GUI over apps Lisa builds and signs, installing through the ADR-0020
channel that already exists. No new trust model and no new threat surface:
it is a face on `lisa apps`. It may ship before anything below.

It must not present itself as a general app store, and must not carry a
"third-party apps coming soon" affordance. CLAUDE.md rule 10: never
document intent as behaviour.

### 2. Manifest as an enforced contract — the real prerequisite

A manifest's declared capabilities become a contract the bus checks, not
a label the app writes about itself:

- The tier an app declares is a **ceiling it asks for**, not a permission
  it holds. The bus resolves the effective tier and refuses anything above
  it, per tool, per call.
- A tool absent from the manifest is not callable, even if the app's MCP
  server offers it. The registry is the authority, not the socket.
- Egress is declared and enforced by the same mechanism that already
  enforces it for daemons (CLAUDE.md rule 5). An app that did not declare
  network access does not get it — the storefront can then state that as
  fact rather than as a promise.
- The manifest is signed with the package. An unsigned or modified
  manifest is a refusal, not a downgrade.

Acceptance: a deliberately lying manifest — declaring `read` while its
server offers a `destructive` tool, or omitting a tool it serves — is
refused by the bus, with a corpus entry in the guard suite. A rule with no
corpus entry is one nobody will notice regressing (CLAUDE.md 6a).

### 3. Third-party signing and provenance — its own threat model

Who may publish, how a key is established and revoked, what happens to
installed apps when a key is revoked, and what the update path is for an
app whose publisher has gone away. This is deliberately *not* specified
here: it deserves its own ADR written against a real publisher story
rather than an imagined one.

Constraint inherited from ADR-0034: the install, update and recovery
paths may not depend on infrastructure we do not control. A third-party
channel that becomes load-bearing for recovery has broken that rule.

### 4. The storefront proper

By this point it is mostly UI over settled mechanism, and its central
screen — what this app can do to your machine and on your behalf — renders
what the *system* knows, never what the *package says*.

## Consequences

**We give up being early.** A first-party catalog is a smaller thing to
show than an app store, and it will read as less ambitious. That is the
honest state of the system, and shipping the ambitious version on top of
declared-not-verified capability would make Lisa's central claim — that
guardrails are deterministic code the model cannot reach — false at
exactly the boundary where users would be trusting it most.

**The manifest work is useful whether or not a storefront follows.**
Enforcing declared capability strengthens the apps we already ship; #212
and #214 were both cases of an app reaching past what its manifest
described, in a system where nothing was checking.

**"Coming soon" is not available to us.** A storefront that lists nothing
third-party, with a placeholder, is a promise rendered as a feature.

## Alternatives considered

**Storefront first, trust later.** Rejected. The claim a storefront makes
is the claim we cannot currently back, and retrofitting enforcement under
a shipped distribution channel means breaking installed apps or
grandfathering them — and a grandfathered app is a permanent hole with a
sympathetic story attached.

**Flatpak, and inherit its sandbox.** Not rejected on merit and worth
revisiting for §3: Flatpak solves isolation and has a real permission
model. It does not solve *agent-surface* capability, which is the axis
Lisa is distinctive on — a Flatpak permission says nothing about which
tools a model may call through the Agent Bus. If we adopt it, it composes
with §2 rather than replacing it.

**Trust the manifest, and rely on review.** Rejected by the evidence in
Context: our own reviewed code carried 27 defects, four of them reachable
guard bypasses. Review is how we found them, not a mechanism that stops
them.

---

# Amendment 1 (2026-08-04) — build or adopt, and what "no third party" means

Three questions came up the same day this was accepted. Recording the
answers here rather than in a second ADR, because they are the same
decision seen from different sides.

## 1. Build our own storefront; do not fork GNOME Software

The original text leaned on "GNOME Software exists" as a reason to defer.
That is weak once you notice we intend to own the desktop anyway, so it
needs replacing with a real argument. Here it is:

**GNOME Software's trust model is per-package. Ours is per-capability.**
It answers "is this package signed, and from a repo you trust?" We need to
answer "what can this app do to your machine and on your behalf?" Those
are different data models, and adopting it would mean fighting its schema
precisely where the only interesting screen lives.

Secondary, but real: it is C and PackageKit-shaped, while ADR-0047 commits
our surfaces to GJS/GTK4. A store in the same stack as every other app is
one we can iterate on by copying a file onto the device — the property
that made a five-fix batch verifiable on hardware in one night.

And a fork is a maintenance commitment forever: every upstream release
becomes a rebase. That is justified for the Shell, where we need behaviour
upstream will not take (#208's system-wide gesture is unreachable because
mutter does not grant it to third parties). It is not justified for a
component we would gut.

## 2. `lisa-desktop` is pre-fork — do not plan against it

`lisa-desktop`'s description says it is *becoming* a hard fork of GNOME
Shell. As of 2026-08-04 it holds **extensions and the IME**
(`overlay-extension`, `launcher`, `desktop`, `fcitx5-lisa`), which ride on
stock GNOME Shell. `gnome-online-accounts-lisa`'s own PKGBUILD states
"Nothing is forked" — stock with two `-D` flags.

So no plan may assume fork-only capability. "We will own it because we are
forking GNOME" is a plan resting on a plan, and treating intent as
capability is the defect rule 10 names.

## 3. "No third-party dependency" — the useful distinction

The constraint is real but needs splitting, or it forbids things it should
not:

- **Depending on a SERVICE somebody else operates** — a remote we do not
  control, an index we cannot rebuild, a build farm that can vanish. This
  is what ADR-0034 §7a forbids for install/update/recovery, and what we
  avoid here by choice.
- **Using SOFTWARE somebody else wrote** — Flatpak, systemd, pacman, GTK.
  Lisa is built almost entirely of this. Self-hosting the service while
  using the software is not a compromise; it is the normal answer.

### How elementary OS solved the same problem

Worth copying, because they faced it directly. Their AppCenter:

- **imports SOURCE from a git repository and builds it in a clean
  environment on infrastructure they run** — developers submit a repo, not
  a binary
- runs the **review queue as pull requests**; merging publishes
- ships apps through **their own hosted, curated Flatpak repo**, not
  Flathub
- **curates hard**: native GTK apps that respect system settings, each one
  tested and reviewed before it reaches users

The dependency they refused is the one that matters: **they never accept a
stranger's binary.** They chose to depend on Flatpak (software) and on
GitHub Actions (a service) — and their own remote is the part they kept.

### Lisa's position is structurally stronger, because of ADR-0047

Our apps are interpreted GJS. There is no compile step, so
"build from source in a clean environment" collapses into "review the
source and sign it": **the artifact IS the source.** A reviewer reads what
runs, and a shipped app cannot diverge from the reviewed code the way a
binary can. Reproducible builds are a problem we do not have to solve
because we do not have builds.

That yields the rule for third-party apps, whenever we get there:

1. **Source in, source out.** We accept a repository, review the source,
   and ship the same files. No binaries from strangers, ever.
2. **We host the index and the artifacts.** Signed with the key
   `lisa-keyring` already ships.
3. **Software we did not write is fine; services we do not run are a
   decision.** Any such dependency gets named in the ADR that adopts it.
4. **Capability is enforced, not declared** — §2 of this ADR, unchanged
   and still the gate.

### On Flatpak specifically

Still not rejected. Its sandbox and permission model are real, and it is
self-hostable, so using it need not mean depending on Flathub. But it
solves *isolation*, not *agent-surface capability* — a Flatpak permission
says nothing about which tools a model may call through the Agent Bus. If
adopted, it composes with §2 rather than replacing it, and it does not
change the sequencing.

## Our own apps come first, and that part is unblocked

The first-party catalog carries Mail, Surfer, Preview, Notes, Recorder and
the shell surfaces — apps we build, review and sign, where the manifest
and its enforcement have the same author. It needs none of §2–§4.

It does need the apps channel to work. As of 2026-08-04 it does not
(#239): `lisa apps update` reported success while installing to a path the
launcher never reads, and the channel did not carry `apps/` at all. **A
storefront's Install button would sit directly on that mechanism**, which
is the sharpest possible argument for fixing delivery before building a
face for it.
