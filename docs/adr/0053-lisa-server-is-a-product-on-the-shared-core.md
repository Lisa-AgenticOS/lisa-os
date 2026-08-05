# ADR-0053 — Lisa Server is a product on the shared core, and its first surface is the Assistant as an API

- **Status:** proposed — design only, no code. Supersedes ADR-0052's
  framing of server mode as a flavor; ADR-0052's lineage mechanics
  remain correct for the day Lisa Server earns its own image.
- **Date:** 2026-08-05

## Context

ADR-0052 answered "how is install mode decided" mechanically: an image
lineage, chosen at install. The owner then named what was actually
being asked — Ubuntu is the analogy not because of two ISOs but
because of **an ecosystem**: one core, distinct products, a management
plane. Lisa Server is not "the desktop with the GUI subtracted"; it is
a product with its own reasons to exist:

- the Assistant reachable on a server (API and web, not a GTK window);
- inference as a **service to what the machine hosts** — a website on
  a Lisa server calls localhost and gets local models, cloud models
  through one broker, one key custody, every call ledgered;
- a server agent — the Agent Bus applied to systemd, logs, config;
- a mobile app to manage the machine remotely.

The load-bearing observation is that **the core is already headless**.
That was not planning ahead; it was forced by rule 5 (egress is
architecture) and ADR-0008's hardened units: `lisa-inferenced`,
`lisa-modeld`, `lisa-remoted`, `lisa-contextd` and `lisa-agentd`
cannot reach a session bus, let alone a display. The desktop has
always been the first *client* of a headless substrate. `lisa ask`
proves it daily over ssh.

## Decision

**One core, two products, one management plane.**

- **Core** (lisa-os): kernel and image machinery, the A/B update
  channel, the daemons, the Ledger, the guard, the model store, the
  CLI. Nothing in the core may require a display, and CI's egress and
  unit checks are what keep that true.
- **Lisa Desktop**: the `lisa-desktop-*` family, the apps, the shell
  fork. Already a contractual layer since the 2026-08-05 rename.
- **Lisa Server**: headless boot + the server-only surfaces described
  below.
- **Management plane**: the mobile/remote app and the web surfaces,
  serving both products.

### The first server surface is the Assistant as an API

Not the server agent, not tenant inference — **the Assistant reachable
over HTTP**, because it is the smallest thing that makes a headless
Lisa immediately worth having, it needs no new *capability* (only a
new surface on daemons that already exist), and it forces the network
identity work everything else is blocked on.

It follows the pattern ADR-0015 already used to add the Assistant
window beside the transient overlay: **one headless backend, many thin
frontends.** The web/API Assistant is another frontend, not a second
brain — the same agent loop, the same contextd namespaces, the same
Ledger, the same remoted egress door.

What distinguishes this endpoint from any other OpenAI-compatible
server — and the reason it is a product rather than a convenience:

| Anyone can run | Lisa Server adds |
|---|---|
| an inference endpoint | per-app durable context (contextd namespaces) |
| a chat UI | an append-only Ledger: every call attributable |
| an API key in each app's env | ONE broker, one custody point (remoted) |
| unbounded tool access | agentd's tiers — a caller can be read-tier by construction |

A governed inference substrate, not an inference API.

### Sequencing (each step is a product increment, not a refactor)

1. **Headless mode as a boot profile** — same image, mode selects the
   boot target; no compositor runs. Reversible with one command, ships
   almost immediately. ADR-0052's separate `lisa-server` **lineage**
   is minted when Lisa Server has features that justify its own
   download page — not before, because a second lineage doubles both
   the image build and the A/B test matrix.
2. **The Assistant as an API + web surface** (this ADR's first
   deliverable), gated on network identity below.
3. **Tenant inference** — what the machine hosts may call the local
   endpoint under policy.
4. **The server agent** — Agent Bus tools for systemd/journal/config.
5. **The management app** — the remote surface over the same identity
   layer as (2).

## The two open design problems, named because everything waits on them

**1. Network identity (blocks 2, 4, 5).** ADR-0033 is absolute:
identity comes from the transport — the broker-assigned peer name,
peer credentials, `/proc/<pid>/exe`. **None of that exists across a
network.** A phone and a hosted web app are the first clients that are
not local peers. This needs its own ADR before any remote surface
ships: device pairing, per-device keys, the WireGuard private edge of
ADR-0031, and an explicit statement of what a remote caller may never
do regardless of how it authenticated. Until then, remote surfaces
bind to loopback and are reached over ssh.

**2. Tenant inference policy (blocks 3).** When a hosted site calls
the local endpoint, "who is asking" is a tenant, not a person. Needs:
quotas, QoS between the owner's own Assistant and hosted workloads
(the §5.1 scheduler already preempts, but has no notion of tenants),
which providers a tenant may reach, and — most sharply — **rule 6
applied to a hosted site's content**. A website's own data is exactly
the untrusted provenance that must never trigger a privileged tool.
Prompt injection through hosted content is the server-world form of
the attack the desktop guard was built for; the guard's vocabulary
extends to inference itself or the product ships with a hole.

## Consequences

- The desktop/core seam gains a second consumer, which keeps it
  honest: a `lisa-desktop-*` dependency leaking into a core package
  breaks the headless boot loudly.
- The GJS Assistant stops being *the* Assistant and becomes one
  frontend; its backend contract becomes public API, and public API
  is expensive to change. Version it deliberately from the first
  commit.
- A mobile app is a different platform from the device, so ADR-0047's
  one-toolkit rule (GJS on Lisa) does not govern it — the toolkit for
  a companion app is an open, separate choice.
- Two products means two support surfaces, two docs sets, and a
  website that must tell people which one they want. That cost is
  real and is the reason step 1 is a boot profile rather than a
  second image.
