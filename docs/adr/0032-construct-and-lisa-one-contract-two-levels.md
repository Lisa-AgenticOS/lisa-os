# ADR-0032: Construct and Lisa — one contract, two levels

- **Status:** proposed — design only, no code. The shared contract
  (manifest, provenance vocabulary, Ledger event shape, tokens) is defined
  on the Lisa side only.
- Date: 2026-07-26
- Relates: PLAN §5.4 + Appendix B (MCP manifest), §5.8 (apps), §5.12
  (Flutter lane), ADR-0004, ADR-0030 (the ontology section)

## Context

Lisa's idea came from Construct (`~/Construct`), which is ours. Construct
is an application platform — Spaces built as Vue 3 plugins, a shared
Operator, a graph-backed memory, a Space Store and an SDK. Lisa is the
same idea expressed one level down: **what Construct does as an app,
Lisa does as an OS.**

That relationship needs writing down, because two products from one team
drift by default. The concepts diverge, the vocabulary diverges, and
eventually there are two SDKs describing the same thing differently.

The two are also genuinely complementary rather than redundant:

- **Construct's strength is reach.** It installs in a minute, runs
  anywhere, and hosts third-party Spaces. Lisa needs a USB stick,
  compatible hardware, and — today — an Apple codec patchset and an
  amdgpu firmware blob for one iMac. That friction gap is enormous and is
  not closed by better engineering. Construct is the on-ramp.
- **Lisa's strength is enforcement.** A platform can *offer* per-app
  memory, permissions and an audit trail; an OS can *enforce* them. A
  Space cannot be prevented from something by the plugin layer it runs
  inside, whereas a Lisa app genuinely cannot reach the context fabric
  without a portal grant, or make a privileged call without passing the
  tier machinery.

## Decision

### 1. Share the contract, not the code

The view layers are different by design and will stay that way. Vue is
correct for Construct: a platform that must run everywhere, iterate fast,
and host third-party code wants a web view layer. Flutter is correct for
Lisa: one toolchain across x86_64 and aarch64, real native windows,
GPU-accelerated rendering, no browser engine per app on a box already
spending its memory on model weights.

**Consequence to state plainly, because it is tempting to wish
otherwise: a Space does not port to Lisa.** A Vue Space and a Flutter app
share no code and never will. Anyone claiming "write once, run on both"
is selling a rewrite.

What ports is narrower and far more durable:

| ports | does not port |
|---|---|
| the manifest: identity, tools, tiers, input schemas, undo | the view layer |
| the provenance vocabulary | components, routing, state management |
| the Ledger event shape | the runtime |
| design tokens | anything rendered |

### 2. The contract is defined in Lisa

Not out of primacy — because it is easier to **relax a constrained spec
when hosting it on a permissive platform than to retrofit enforcement
onto a permissive one**. This is the same argument ADR-0029 used for
keeping the `Deny` class small and absolute, and ADR-0030 generalised
into the ontology section: the logical layer can only reason over what is
typed and declared.

Lisa's Agent Bus manifest (§5.4, Appendix B) already carries reverse-DNS
identity, per-tool tiers, JSON input schemas and undo declarations.
Construct honours the same declaration; Lisa is where it is *enforced*
rather than merely respected.

Which yields the story worth telling: **the same declaration runs in both
places, and gets more when it runs on Lisa** — real portal grants,
per-app durable context, no-egress local inference, and an append-only
Ledger the Space cannot write around.

### 3. One token source, two emitters

Construct already has a real design system with a theme source
(`design-system/src/themes.ts`, plus a theme switcher and themes page).
`libs/lisa_ui` is currently a single Dart file — so there is nothing to
throw away, and generating it is cheap *now* and expensive in a month.

Colour, type scale, spacing, radii and motion are technology-neutral.
Generate `lisa_ui`'s Flutter theme from `themes.ts` rather than
hand-maintaining a second palette that drifts. That buys a Space and a
Lisa app that look and behave like one product without sharing a runtime
— the only kind of coherence that survives two view layers.

### 4. What Lisa has that a platform structurally cannot

Recorded because it should drive what Lisa builds next, rather than
chasing Construct's feature list:

- **Lisa can author its own Spaces at runtime.** The Forge already runs
  the whole Flutter lane end to end — scaffold a `lisa_ui` app,
  `flutter analyze` as verifier, `flutter build linux --release`, install
  the bundle, write the `.desktop`. Construct's Space Store is populated
  by humans holding an SDK; Lisa's app set can be populated by the agent,
  on the machine, while it is running.
- **Lisa can extend the system, not just add an app.** A timer, a path
  unit that watches a folder, a udev rule, a D-Bus service. "Summarise
  new PDFs dropped in this folder" is not an app at all. A plugin host
  cannot emit it. (Gated by blast radius — ADR-0031 §5.)
- **Lisa can serve what it makes** (ADR-0031 §4). Any platform hosts your
  output on its own infrastructure, because that is what being a platform
  means.

### 5. Two UI stacks inside Lisa, with a stated boundary

Lisa itself now has two, and "which do I write?" is the first question
any contributor will ask:

- **GJS/GTK** for **shell surfaces** — overlay, launcher, Assistant,
  Ledger app, Settings panel. GNOME Shell extensions must be GJS, and
  these integrate with the session.
- **Flutter, via `lisa_ui`** for **applications**, including everything
  the Forge produces.

The line is *shell surface versus application*, not preference.

## What was rejected

- **A shared runtime.** A cross-compiling abstraction over Vue and
  Flutter is a large permanent tax paid for a small one-time benefit.
- **Defining the contract in Construct.** Permissive specs do not grow
  enforcement later; constrained ones relax easily.
- **Merging the products.** Different substrates, different friction,
  different buyers. The two-level structure is the strategy, not an
  accident to be cleaned up.

## Consequences

- One manifest specification, versioned in this repo, honoured by both.
- `lisa_ui` becomes a generated theme plus hand-written widgets, and gains
  a build step. Cheap today, so do it before `lisa_ui` grows.
- Positioning gets an answer that lives in the repo rather than in
  someone's head: Construct is where the idea runs everywhere; Lisa is
  where it is enforced, where it can build itself, and where it can serve
  what it builds.
