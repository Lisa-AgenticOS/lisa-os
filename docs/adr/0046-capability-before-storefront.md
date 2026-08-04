# ADR-0046 — Capability before storefront: what must be true before Lisa distributes somebody else's app

- **Status:** Accepted
- **Date:** 2026-08-04
- **Supersedes:** nothing. Extends ADR-0020 (apps channel), ADR-0039 (repo
  split and `[lisa]`), ADR-0029/0030 (guardrails), ADR-0033 (identity from
  the transport).
- **Anticipated by:** ADR-0023, which names third-party app distribution as
  a *future* goal rather than a current one.

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
