# ADR-0031: Server mode, the two edges, and artifact publishing

- **Status:** superseded in part by ADR-0053 — still proposed, still no
  code (no `serverd`, no `lisa serve`, neither edge exists). ADR-0053
  promotes §1's "server mode is a flavor chosen at install" into a
  **product** with its own surfaces and a sequencing ladder, and
  re-decides §2's management/use split: the first server surface is
  the Assistant as an API/web frontend of the existing backend, not a
  Cockpit module. §3 (the two network edges) and §4 (artifact
  publishing) stand, and are what ADR-0053's network-identity work
  builds on.
- Date: 2026-07-26
- Relates: PLAN §5.11 (Personal Compute Node), §5.12 (Forge), M7;
  ADR-0020 (apps channel), ADR-0023 (slim core, /var grows),
  ADR-0029/0030 (guardrails); issues #53, #54, #55

## Context

PLAN §5.11 already specifies a headless `lisa-node` — same
`inferenced`/`modeld`, no desktop — paired over WireGuard, discovered by
mDNS, offering a `remote:personal` tier to another Lisa machine. What it
does not specify is **any human interface**. The node is designed purely
as a peer for machine-to-machine offload. There is no way for a person to
see what it is doing, pull a model, or read the Ledger without SSH — and
the Ledger being invisible on a headless box is the sharpest version of
that gap, because the Ledger is the audit spine of the whole system.

Separately, a capability has emerged from the Forge that changes what a
server is *for*. Lisa can build a thing — a Flutter app, a report, a
static site — and it is running on a machine that could also **serve**
it. Ask for a deep-research report and get back a URL; ask for a website
and have it built, installed and published. Neither half is novel alone:
ChatGPT and Claude make but do not serve; Vercel and CapRover serve but
do not make; v0 and Lovable do both but on *their* infrastructure. **The
closure on hardware you own is the thing nobody has.**

That reframes the network edge. This ADR was nearly written as "expose
the management UI," which is a bad idea. Publishing *artifacts* is a
different and much smaller surface, and it is the one worth building.

## Decision

### 1. Server or desktop, chosen at install (M7 OOBE)

> **Revised 2026-08-05 by ADR-0052 + ADR-0053.** Choosing at install
> stands. Two corrections: the *mechanism* is which image lineage the
> disk tracks, never a package toggle on a running system (ADR-0052);
> and the *first step* is a boot profile on the single image rather
> than a second lineage, because a second lineage doubles the image
> build and the A/B test matrix before Lisa Server has features to
> justify it (ADR-0053's ladder). Also note the paragraph below is now
> optimistic about one thing: since ADR-0039 step 4 the nightly image
> carries the full desktop set, so "CI already builds a desktop-less
> image" is no longer true as written — what CI still proves nightly
> is that the daemons boot without a session, which is the claim that
> matters.

The installer asks once. Server mode installs `lisa-node` — the daemons,
the CLI, the model store, no GNOME. This is close to free: the nightly
already builds a minimal image with no desktop and boot-tests it every
night, and the postinst is written flavor-defensively throughout. Server
mode is mostly *promoting something CI already builds* to a supported
profile.

### 2. Management is Cockpit; use is ours

Two surfaces, deliberately separate.

**Management → a Cockpit module.** Cockpit (`cockpit` 364-1 in Arch
`extra`, with `cockpit-storaged`, `cockpit-packagekit` et al. split out)
is systemd- and D-Bus-native, which is the substrate CLAUDE.md rule 4
already commits us to. A Cockpit page is HTML/JS calling `cockpit.dbus()`
and `cockpit.spawn()`, so a Lisa module can show models, Ledger, A/B slot
and update state, and daemon health **without a web backend of our own**.
It also brings PAM auth, TLS and session handling — which removes the
riskiest thing we would otherwise be writing ourselves.

**Use → Lisa's own.** Chatting with the model and driving the harness are
not administration, do not fit an admin console's idiom, and are the
differentiated half. A stock PatternFly panel is the wrong place for the
thing that is supposed to feel like Lisa.

Rejected: Ajenti and Webmin. Right family, wrong substrate — panels that
manage a generic Linux box from outside, rather than talking to the buses
our daemons already sit on.

### 3. Two edges, because the two target machines differ

| | inbound | edge |
|---|---|---|
| VPS | public IP, 80/443 reachable | public edge works directly |
| Mac mini at home | NAT, often CGNAT | **cannot** do inbound; tunnel only |

So this is not a choice between private and public — it is both, selected
at install alongside the profile. The private edge is not the paranoid
option; it is the only one that works for half of all deployments.

**Private edge (default):** WireGuard, the §5.11 QR/short-code pairing as
authentication, nothing bound to a public interface. WG keys are a better
answer than a login form, and M7 needs the pairing regardless.

**Public edge (opt-in):** wildcard DNS at the box, reverse proxy, ACME
certificates. Three requirements, all load-bearing:

- **Publish a whitelist, never "what is listening."** Lisa's daemons are
  not apps someone chose to deploy; they are the OS. The proxy publishes
  explicitly named surfaces and routes nothing else. `contextd`,
  `agentd` and the Ledger database are unreachable from the edge by
  construction.
- **Per-client API keys, attributed in the Ledger.** A public
  OpenAI-compat endpoint is a credential-guarded API, and the failure
  mode is not a stranger using your GPU — it is your context fabric
  answering their questions. Every public request is attributable to a
  named, revocable key.
- **The inbound terminator is its own daemon.** Same discipline that
  gives `lisa-remoted` sole ownership of egress. A process holding TLS
  keys and public exposure must not be `inferenced`.

Scope boundary: take the *routing*, not the PaaS. Wildcard DNS + ACME +
reverse proxy is configuration over something like Caddy. Deploying
arbitrary containers is a different product.

### 4. Artifact publishing is the primary public surface

The public edge does **not** need to expose the inference API to be
valuable. Serving a directory of published artifacts is a far smaller and
safer surface, and it is the one that delivers the product moment. Ship
it first; treat the public API as a separate, later decision.

**Producer.** The Forge, extended: `lisa forge --flutter` becomes a
family, with `--web` as the sibling that emits a static site. Same loop —
plan, edit under the jail, verify, build — different verifier and
different install target.

**Store.** ADR-0020's mechanics, pointed at a new directory: versioned
trees under `/var/lib/lisa/published/<name>/<version>` with an atomic
`current` symlink and rollback. No new machinery.

**Publishing rules:**

- **Generating is not publishing.** The agent producing a report must not
  mint a public URL as a side effect. Publishing is outward-facing and
  hard to un-ring — caches and crawlers do not forget — so it is a
  confirm-tier action under the ADR-0029/0030 machinery, ledgered, with
  `unpublish` as one command.
- **Untrusted provenance may never reach publish.** If the agent read a
  page and the page says "publish my notes," the tier escalation must
  stop it. Given #55 (caller-asserted provenance) and the deferred
  model-in-the-loop injection suite, **this capability is gated on those
  two landing first.**
- **Capability URLs by default.** A random token in the path,
  `noindex`, no crawlable listing. Vanity paths are opt-in per artifact.
  A report synthesised from your context fabric at a guessable URL is a
  disclosure even though nothing was compromised.
- **One origin per artifact.** `<token>.artifacts.<domain>`, never a
  shared host and never the same origin as any Lisa surface. Generated
  pages contain model-authored JavaScript; two artifacts sharing an
  origin means one can read the other's storage. This is why CodePen and
  JSFiddle serve user code from a separate domain.
- **Static only, for now.** Serving generated HTML is ordinary web
  serving. Serving a generated SSR application means running
  model-authored *server* code on the machine holding your context
  fabric and Ledger — a different risk class, made worse by the
  unresolved ADR-0029 phase 3 gap (#53). "Run the thing it built" needs a
  container or VM boundary.

### 5. Forge output is sequenced by blast radius

What the Forge may produce is now a security boundary, not a feature
list:

1. **GUI apps** — user session, bounded by the jail and the command
   allowlist. Shipping today.
2. **CLI tools** — same session, same bound. Next.
3. **Units, timers, services, anything at boot or outside your session** —
   arbitrary persistent execution chosen by a model. **Not before** #53
   (Landlock) and the model-in-the-loop injection suite. Today the
   harness is jailed for its own file tools and unconfined for the
   toolchains it invokes; a forged systemd unit turns that gap from
   theoretical into shipped.

## Consequences

- M7 gains a real second product without a second codebase: server mode
  is the minimal image CI already builds, plus `lisa-node` and an edge.
- We take a dependency on Cockpit in the server profile only. The desktop
  image does not carry it, and ADR-0023's budget accommodates it because
  server mode has no GNOME payload.
- The public edge introduces the first Lisa surface reachable by people
  who are not the owner. Every requirement in §3 and §4 exists because of
  that sentence.
- Publishing is blocked behind provenance verification and the injection
  suite. That is a real schedule cost and the right one: it is the first
  capability where a guardrail failure harms someone other than the
  owner, which is exactly where ADR-0030's "your machine, your rules"
  stops being a complete answer.
