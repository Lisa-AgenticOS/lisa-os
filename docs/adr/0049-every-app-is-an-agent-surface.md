# ADR-0049 — Every app is an agent surface: install is the grant, the tier is the gate, the registry is the authority

- **Status:** accepted, not implemented — the decision stands and the
  mechanism is largely unbuilt. What exists is the table in §"What exists
  today" (manifests, tiers at the bus, `lisa tools`, the grant log). Not
  built: registration at install and deregistration at uninstall, the
  registry as a stateful authority rather than a startup scan, per-app
  skills, and stored grant state (#240).
- **Date:** 2026-08-04
- **Extends:** ADR-0009 (Agent Bus core — tiers enforced in the bus state
  machine), ADR-0013 (the MCP dispatcher), ADR-0025 (one agent loop;
  Skills as `SKILL.md`), ADR-0029/0030 (guardrails, and the boundary),
  ADR-0033 (identity comes from the transport), ADR-0036 (trigger
  classes), ADR-0046 + Amendment 1 (capability before storefront),
  ADR-0047 (one toolkit), ADR-0048 (core vs. store apps).
- **Related:** PLAN §5.4 and Appendix B (the manifest), Appendix C (the
  provenance envelope); issues #240, #219, #97, #56, #57, #147.
- **Supersedes:** nothing. It names the thing the earlier decisions were
  each solving a corner of.
- **Claims:**
  - `symbol:fn apply_tier_floor@daemons/agentd/src/manifest.rs` — tiers at the bus
  - `path:apps/mail/app.lisaos.Mail.json` — a per-app manifest
  - `symbol:fn tools_cmd@cli/lisa/src/agent.rs` — `lisa tools`, the discovery surface

## Context

Every desktop is adding an assistant. The ones shipping now are a chat
window bolted to the side of a system that does not know what its apps
can do: the assistant is a *feature of the OS*, and the apps are opaque
to it.

Lisa's difference is one sentence: **the unit of capability is the app,
and installing an app is how the machine learns to do a new thing.** An
app does not merely have an assistant in it. An app *is* a set of things
the assistant can now do — tools it can call, and workflows it knows how
to run — and those enter the model's world at the moment the person
chooses to install it, and leave when they uninstall.

That claim is only interesting if it is uniform. If Mail and Photos get
their capability through a private back door and a stranger's app gets it
through a governed one, then the governed path is a compliance exercise
and the interesting path is untested. **First-party apps are governed
exactly as third-party apps are.** We are the first user of our own
mechanism, which is the only reliable way to find out whether it works.

ADR-0046 already named this as the storefront's distinctive screen —
"what this app can do to your machine and on your behalf" — and gated the
storefront on capability being enforced rather than declared. ADR-0048
made "core vs. store" a boundary with a test. This ADR is the layer under
both: *what registration means, what a grant covers, and who is allowed
to answer the question "what exists?"*

## What is broken today, measured

Issue #240, on the reference iMac (image 20260804.76):

```
$ lisa tools
app.lisaos.Browser::read_page      —  Read the page open in the current Browser tab…
app.lisaos.Browser::get_selection  —  Read the text the user has selected…
app.lisaos.Browser::screenshot     —  Screenshot the visible part…
```

**`app.lisaos.Browser` does not exist.** It was renamed to Surfer months
ago — no package, no socket, no process. The model is still told it has
three Browser tools, because `~/.local/share/lisa/manifests/app.lisaos.Browser.json`
was written once and never reaped. A tool the model can see is a tool it
will try: every phantom entry is a guaranteed failed call, a wasted turn,
and a model reasoning about a machine that is not the one it is on.

Three sources disagree about what exists, and none of them is
authoritative:

1. **Manifest files on disk** — `daemons/agentd/src/main.rs:71` scans the
   manifest directories *once, at daemon start*. Nothing removes a file,
   nothing notices one appearing, nothing re-scans.
2. **Socket presence** — `/run/user/1000/lisa/mcp/*.sock`. #219 showed
   this wrong in the other direction: a killed app leaves a socket that
   refuses connections while the bus treats presence as availability.
   Confirmed separately in Surfer and Mail.
3. **The running process** — the only one that is actually true, and the
   one nothing consults.

They are not merely stale relative to each other; they are independently
populated. In the same capture, Mail's tools were advertised with no
`app.lisaos.Mail.json` in the user manifest dir at all, while notes had
both a manifest and a socket.

There is a fourth disagreement, found while writing this and worth as
much as the other three:

```
os/packages/lisa/PKGBUILD:391  install -Dm644 apps/preview/app.lisaos.Preview.json \
os/packages/lisa/PKGBUILD:392      "$pkgdir/usr/share/lisa/apps/app.lisaos.Preview.json"
```

Surfer, Mail and notes install to `/usr/share/lisa/manifests/`. Preview
installs to `/usr/share/lisa/apps/` — **a directory nothing reads.**
`SYSTEM_MANIFEST_DIR` is `/usr/share/lisa/manifests`
(`daemons/agentd/src/main.rs:17`), and a repo-wide search finds exactly
one reference to `share/lisa/apps`: the line that writes it. Preview is a
shipped, core app (ADR-0048 §5) whose declared tools have never reached
the model, and nothing anywhere reported that. It is #240's defect
mirrored: a real app that is invisible, next to a dead app that is
visible.

This is the state of "the registry" that PLAN §5.4 describes as
*"maintains the registry of installed servers"*. There is no registry.
There is a startup directory scan, plus packaging conventions, plus
whatever files happen to be lying around.

## Decision

### 1. Every app is an agent surface: tools **and** skills

An app declares two things, not one:

- **Tools** — typed actions with tiers, schemas and undo (PLAN Appendix
  B; this exists).
- **Skills** — `SKILL.md` workflows in the ADR-0025 shape, taught by the
  app, loaded on demand. An app knows how to do things with itself that
  no generic model knows; a skill is where that knowledge lives. This
  does not exist per-app today; skills are a system-wide search path
  (`cli/lisa/src/skills.rs:29-40`).

The same mechanism serves Mail, Photos and Notes as serves a stranger's
app. No first-party bypass, ever. If a first-party app needs something
the mechanism does not offer, that is a defect in the mechanism.

### 2. Install registers; uninstall deregisters

A background service initialises an app's tools and skills at install
time. From that moment the model can see them; after uninstall it cannot,
and the registration is gone rather than lingering as a file.

**The grant happens at install, and this is deliberate** — chosen over
prompting at first use. It works because of what is being granted:
**install grants *existence*, not *action*.** Saying yes to Mail means
"Mail's tools may appear in my assistant's catalogue", not "Mail's tools
may run without asking". Every call is still gated by §3.

### 3. Tiers govern every call, forever

Registration decides what enters the model's world. **The tier decides
what happens when it is exercised** — read acts silently and ledgered,
write raises a confirmation chip, destructive raises a modal with a typed
diff (PLAN §5.4; enforced in `daemons/agentd/src/bus.rs`, not by app
goodwill).

Say the obvious objection out loud, because it is the strongest one:
**install-time consent alone is famously weak.** Android shipped it for
five releases and moved to runtime prompts in 6.0, because a permission
list on an install screen is click-through, and a click-through is not
consent. Anyone who has watched a person install an app knows this.

That lesson does not bite here, and the reason is precise: **on Android,
install-time consent was the *only* gate — granting the permission was
granting the action.** Here it is the gate on *what exists*; the tier is
the gate on *what happens*, and it fires at every call for the life of
the installation. The person who clicks through Mail's capability page
without reading it still gets a modal before mail is deleted.

The converse is also why we do not collapse the two into a first-use
prompt. A prompt at first tool use asks the person to make a security
decision at the moment they are least equipped to make it — mid-task,
with no context about the app as a whole, in a dialog they want gone. It
adds friction at the precise place friction produces reflexive clicking,
and it removes the one moment when the person *is* thinking about this
app as a whole and comparing it to others. **Naggier is not safer.** The
tier gate is what carries the safety, and it is not a prompt about the
app — it is a prompt about *this action, now*, which is a question a
person can actually answer.

### 4. An update that widens capability is a new grant moment

The registry stores what was granted. On update it compares the new
declaration against the stored one, and **capability that was not in the
previous grant is registered INERT** — present, visible, refused on call
— until the person agrees to it.

Without this, permission creep through updates is free: ship a useful
app declaring two read tools, and add a destructive tool in v1.2 that
nobody was asked about. This is not hypothetical; it is the standard
attack on every install-time consent model that ever shipped, and it is
the reason install-time consent has a bad name.

This is the concrete reason **the registry must be an authority with
state, not a directory of manifest files.** A file on disk cannot answer
"what did the person agree to?", because the file *is* the new
declaration — comparing it to itself is not a comparison. Something must
have remembered the old answer.

### 5. The registry is the sole authority on what exists

`lisa-agentd` owns one registry. A manifest file is an **input** to
registration, never a source of truth (ADR-0046 §2 said the same thing
about the socket: "the registry is the authority, not the socket").

Two states that today collapse into one and must not:

- **Installed but not available** — registered, app not running. The
  model may see the tool and the bus may activate the app, or say
  plainly that it is not available. This is a normal state.
- **Not installed** — not registered. The tool does not exist and the
  model is never told about it.

Neither of these is "a file happens to be present in a directory". A
registered app whose payload is gone is deregistered, not left
advertising.

The identity rules carry over unchanged: which app a socket belongs to
comes from peer credentials and `/proc/<pid>/exe`, never from what the
message says it is (ADR-0033). Registration says an app *may* serve
`app.lisaos.Mail`; it does not make whoever connects to the socket be
Mail.

## The question this ADR had to answer: do skills carry tiers?

**No. Skills carry provenance instead — and today they carry neither.**

The argument, then the evidence.

**A skill is instructions; a tool is an action.** A skill that chains
three write-tier tools *feels* like a privileged thing, but each of those
three calls is still an individually tiered tool call through the bus,
with its own chip or modal. A skill therefore cannot do anything its
tools could not do without it. Giving a skill a tier of its own would
mean either asking twice for the same action, or — much worse — asking
*once, up front*, and treating that as consent for the calls inside,
which is precisely the install-time-consent-as-the-only-gate failure §3
rejects.

Verified against the code, because the argument is only worth having if
the loop actually behaves this way:

- A skill's `tools:` frontmatter is an allowlist that **narrows** and can
  never widen. `harness_core::Skill::allowed_by` intersects across active
  skills (`libs/harness-core/src/skill.rs:136`), explicitly so that
  activating an unrestricted skill cannot restore the full tool set, and
  the loop enforces it before dispatch
  (`libs/forge-harness/src/agent.rs:578`, issue #57 — it was parsed and
  ignored until then).
- Bus tools reach the model only through `read_tier_tools`
  (`libs/bus-tools/src/lib.rs:38`), which offers **read tier only** and
  drops any row whose tier is absent rather than defaulting it. A skill
  does not change that catalogue.
- Everything privileged parks in `daemons/agentd/src/bus.rs`, whose
  outcome the loop is forbidden to answer for itself
  (`libs/bus-tools/src/lib.rs:77` — "this must never answer its own
  confirmation").

So a skill has no privilege of its own, and the tier machinery is not
reachable from inside a skill body. By ADR-0029's first test — *can the
untrusted party influence it?* — a skill is on the wrong side of the
boundary to be a privilege carrier at all.

**But that is the answer to the wrong worry.** The real hazard is not
that a skill is powerful. It is that **a third-party skill is
attacker-controlled text loaded into the model's context.** It is a
prompt-injection surface, and it arrives with better standing than a web
page: retrieved into the prompt on the strength of a name match, by a
component the person installed on purpose.

The asymmetry is visible in the code. Tool *results* already carry
provenance and taint the chain:
`bus_tools::result_is_web_tagged` (`libs/bus-tools/src/lib.rs:101`)
parses the result JSON — deliberately not a string search, so a page that
merely *contains* `"provenance":"web"` cannot taint by mention — and sets
a one-way `web_tainted` flag, after which every subsequent call carries
`web` in its chain and agentd escalates anything privileged (#146 phase
4). Skill *bodies* have no equivalent. `harness_core::Skill` holds
`name`, `description`, `tools` and a **private** `path` with no accessor
(`libs/harness-core/src/skill.rs:39-48`): after loading, nothing
downstream can even ask where a skill came from. The body is read
verbatim (`Skill::body`) and, when `load_skill` exists, will enter the
prompt with the same standing as the system policy.

Hence the decision:

**Skills get provenance, not tiers.**

1. A skill's origin travels with it in the registry — which app declared
   it, from which package, at which grant. `Skill` gains an origin the
   loop can read; today it structurally cannot.
2. A skill body from a non-system origin is fenced as untrusted content
   in the prompt envelope, the way retrieved content already is (PLAN
   Appendix C, `[context source=… trust=untrusted]`).
3. CLAUDE.md rule 6 applies unchanged: untrusted provenance never
   triggers a privileged tool call without escalated confirmation. A
   write-tier call whose chain includes a third-party skill escalates,
   exactly as a web-tainted chain does.
4. **App-declared skills are namespaced by `app_id` and cannot shadow a
   system skill.** This is a real gap, not a hypothetical: skill
   resolution today is *first directory wins* with the user-writable
   `$XDG_DATA_HOME/lisa/skills` ahead of the packaged set
   (`cli/lisa/src/skills.rs:29-40`) — the exact opposite of the manifest
   precedence that issue #97 established, where the system directory is
   unconditionally first and a user file may add an app but never
   redefine one.

Point 4 deserves its rule-6a reading, because the two cases look
identical and are not. **An owner writing a skill into their own home
directory is the owner acting on their own machine** — a guardrail there
would sit between a person and their computer, which ADR-0030 says is not
what guardrails are for. **An installed app dropping a file into that
same directory is not the owner**, even though the bytes land in the same
place. Provenance is what distinguishes them, which is the whole reason
it must be carried rather than inferred from a path.

## What exists today, and what this ADR decides for the future

Stated separately because conflating them is the defect CLAUDE.md rule 10
names.

**Exists, verified 2026-08-04:**

| | where |
|---|---|
| Apps expose MCP tools over per-app unix sockets | `libs/mcp-bus`, `apps/*/lib/mcp*.js` |
| Manifests in the Appendix B shape, with tiers | `apps/mail/app.lisaos.Mail.json` and siblings |
| `lisa tools` lists them as `app.lisaos.X::tool` | `cli/lisa/src/agent.rs:233` |
| Descriptions carry untrusted-provenance warnings | Mail's `search_mail`, Surfer's `read_page` |
| Tier floor raises a lying tool name | `daemons/agentd/src/manifest.rs:234` (#56) |
| A user manifest cannot redefine a system app | `daemons/agentd/src/registry.rs:109` (#97) |
| Tiers enforced in the bus; loop sees read tier only | `bus.rs`, `libs/bus-tools/src/lib.rs:38` |
| A consent surface for chips and modals | `shell/consent/lisa-consentd.js` |
| Skills as `SKILL.md` with an enforced tool allowlist | `libs/harness-core/src/skill.rs`, `forge-harness/src/agent.rs:578` |
| An append-only per-(app, scope) grant log | `portals/xdg-desktop-portal-lisa/src/grants.rs` |

**Not built — what this ADR decides:** registration at install and
deregistration at uninstall; the registry as an authority with state
rather than a startup scan; per-app skills; stored grant state for an
app's agent surface; the update-widens-capability comparison and the
inert state; skill provenance; *installed but not available* as a
distinct, reported state.

The last row of the "exists" table is the most useful thing in it. The
portal already keeps consent as an **append-only action log** keyed by
(app, scope), with `allow`/`deny`/`revoke` and effective state derived
rather than stored, plus `deny_once` recorded specifically so
prompt-until-they-click-yes is detectable (#113). That is the same shape
an agent-surface grant needs, written by us, tested, and in the tree. The
grant store for §2–§4 is a second consumer of a known-good design, not an
invention.

## Consequences

**The App Store's central screen becomes real.** ADR-0046 said the
distinctive page is "what this app can do to your machine and on your
behalf". This ADR is what lets that page render two sentences nobody else
can render:

- *"Installing this adds these tools and these skills to your agent"* —
  at install.
- *"This update wants to add these tools"* — at update, with the
  difference shown, and the new capability inert until the answer.

Neither is renderable from a package manager's metadata, because neither
is a property of the package. They are properties of the registry, and
GNOME Software's data model has no place to put them (ADR-0046 Amendment
1 §1). This is the concrete cash value of building our own store.

**We take on state we have to keep correct.** A registry with grant
history is a thing that can be wrong, can be corrupted, and must survive
A/B updates. It lives in the user's home, which is a real partition and
already survives updates (ADR-0034 §7b). The honest cost: today's failure
mode is a stale file advertising a dead app; tomorrow's is a grant record
disagreeing with an installation. The second is rarer and worse, and the
mitigation is that the append-only-log shape makes it auditable rather
than merely wrong.

**agentd needs a live registry, and does not have one.** Registration at
install requires the daemon to learn about an app while it is running.
Today the scan happens once at start (`main.rs:71`), and
`AgentBusTools` snapshots the catalogue at loop construction
(`libs/bus-tools/src/lib.rs:113`). Both are deliberate simplifications
that this decision retires; the snapshot rule for a *running loop* stays,
because changing the tool list under a conversation means the model holds
a spec for a tool that no longer exists.

**Install-time registration widens the prompt every time.** An app the
person never opens still costs catalogue tokens in every turn, and adds a
tool the model may try. That is the direct cost of "install teaches the
machine", and the mitigations are the boring ones — *installed but not
available* reported honestly, and discovery ranking rather than dumping
the whole catalogue (`registry.rs:187` already ranks).

**Uninstall becomes a security-relevant operation.** If deregistration
fails, the machine is in exactly the #240 state. It needs a test that
asserts removal, not a code path that hopefully runs.

## Alternatives considered

**Prompt at first use instead of at install.** Rejected in §3: it asks
the security question at the worst moment, trains click-through, and
would still need §4's update comparison to be safe — so it buys nothing
and costs the one screen where a person is actually comparing apps.

**Tier the skills.** Rejected above, with the code that shows a skill
cannot exceed its tools. It would also create a false sense of a gate:
a "read-tier skill" that a person approved once still gets to say
anything it likes into the model's context, which is the hazard that
actually exists.

**Keep manifests as the source of truth and just reap the stale ones.**
This fixes #240's symptom and none of its cause. Reaping needs an
authority to decide what is stale; once you have written that, you have
written the registry. And it still cannot answer §4 — a file cannot
remember what the previous file said.

**Let first-party apps skip registration, since we sign them anyway.**
Rejected. It would mean the mechanism every third-party app depends on
is exercised by nothing we run daily, and would be discovered broken by
the least forgiving possible user. Preview's manifest sitting in a
directory nothing reads, unnoticed for as long as it has been, is what
happens when a path is not the path we all use.

## What would reverse this

- **The tier gate turning out to be the weak half.** The whole argument
  for install-time registration is that the per-call gate carries the
  safety. If the injection suite or a field defect shows privileged calls
  reaching dispatch without confirmation, then install-time grant is
  granting action after all, and the model has to move to first-use
  prompting until the gate is trustworthy again.
- **Skills growing the ability to act directly.** If a skill ever
  executes anything other than through an individually tiered tool call —
  an embedded script, a macro language, a direct socket — the
  no-tier conclusion is void the same day. The property to guard is
  "every action a skill causes is a tool call the bus sees".
- **Per-app skills proving to be noise.** If the app-declared skills
  people write turn out to be documentation rather than workflows, the
  cost (context, provenance plumbing, an injection surface) buys nothing
  and skills stay system-scoped.
- **A registry that cannot be kept honest.** If grant state and
  installation state drift in the field despite the log, the honest
  fallback is to derive everything from installation and lose §4 — which
  would mean accepting permission creep through updates, and should be
  argued explicitly rather than arrived at.

## First implementation slice

Not part of the decision; recorded so the next session does not have to
re-derive it. In order:

1. **One authority, live.** agentd owns the registry as state with a
   register/deregister path, and #240's acceptance tests: a renamed app's
   tools vanish, and no tool reaches the model from a manifest file
   alone. Fix Preview's install path in the same change, and add the
   check that would have caught it — a manifest installed to a directory
   the daemon does not search is a build failure, not a silent one.
2. **Liveness reported, not guessed** — *installed but not available* as
   a real state, which closes #219's half from the registry side.
3. **Grant state**, in the shape `portals/…/grants.rs` already proved,
   plus the update comparison and the inert state.
4. **Skill provenance** — an origin on `Skill`, fenced rendering for
   non-system origins, and a corpus entry in `libs/lisa-guard` for a
   skill body that tries to trigger a privileged call. A rule with no
   corpus entry is one nobody will notice regressing (CLAUDE.md 6a).
5. **Per-app skills** last, because they are the piece that has no
   existing consumer to keep honest.
