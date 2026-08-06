# daemons/harnessd — the one agent loop, as a service

Spec: ADR-0025 (one agent loop), ADR-0036 (triggers and trust).
Serves `dev.lisaos.Harness1` on the session bus.

## What it does

Runs the agent loop for every surface that wants an assistant — the
Assistant window, the overlay, `lisa assist`, and later schedules and
event sources. One loop, one answer to "which tools may the model use",
one place the provenance rules live.

```
Run(s prompt, a{sv} options) → (t run_id)
    options: "model" (s), "url" (s), "trigger" (s: prompt|schedule|event),
             "history" (s: JSON [{role, content}]),
             "workspace" (s: an absolute folder path),
             "attachments" (s: JSON [content part, …])
Cancel(t run_id)
signal Tool(t run_id, s name, s detail)
signal Token(t run_id, s delta)
signal Finished(t run_id, b ok, s summary)
```

Shaped like `Overlay1`'s Ask/Token/Finished deliberately: the Assistant
window already renders that vocabulary, so adopting the harness is a
change of destination, not a rewrite.

`url` defaults to `unix:$XDG_RUNTIME_DIR/lisa/inferenced.sock` (or
`$LISA_INFERENCED_SOCKET`) — see "No network at all" below for why it is
a socket and not `http://127.0.0.1:7778`. An `http://…` value still
works for a developer running the daemon outside its unit; under the
shipped sandbox it simply cannot connect.

## No network at all (#288)

This daemon hosts the MODEL, which makes it the one process an injected
instruction is executing inside. Until 2026-08-06 its only network
barrier was `IPAddressDeny=any` + `IPAddressAllow=localhost`, and **that
pair does nothing in a user unit**: an IP firewall is a cgroup BPF
program, which `systemd --user` cannot load. The user manager says so:

```
lisa-agentd.service: unit configures an IP firewall, but not running as root.
```

Measured on the reference iMac with two transient *user* units:

```
IPAddressDeny=any + RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
    -> curl http://example.com   HTTP=200   (reached the world)
IPAddressDeny=any + RestrictAddressFamilies=AF_UNIX
    -> curl http://example.com   rc=7       (blocked)
```

So the model host had unrestricted egress while both gates called it
no-egress. The only directive that confines an unprivileged unit is
`RestrictAddressFamilies=`, a seccomp filter on `socket(2)` — and taking
`AF_UNIX` alone forbids the `:7778` hop outright. The hop therefore
moved rather than the barrier: `lisa-inferenced` already served the same
OpenAI-compatible API on `%t/lisa/inferenced.sock` (the door
`lisa-contextd` has used since #163), so the loop speaks to it there,
via `forge_harness::unix_http`, and the unit now carries
`RestrictAddressFamilies=AF_UNIX`.

The proof that both halves hold, on the device, under the new
directives: `curl http://example.com` → `rc=7`,
`curl http://127.0.0.1:7778/v1/models` → `rc=7`, and a real streamed
`Run()` finishing `(1, true, 'The capital of France is Paris.')`. The
same run against the *previous* binary fails with `backend: io: Address
family not supported by protocol (os error 97)` — which is what makes
the confinement a mechanism and not a comment.

## Attachments (#209)

`attachments` carries OpenAI content parts — the shape `lisa ask
--attach` builds and `lisa-inferenced` accepts as `Content::Parts`:

```json
[{"type": "image_url", "image_url": {"url": "data:image/png;base64,…"}}]
```

When present, the prompt turn goes out as parts with **the person's text
first** — a model handed the image before the question answers the
question it invented for the image. When absent, the turn is a plain
string, byte for byte what it always was.

The parts are **opaque**: the daemon does not re-model a provider's part
schema, it forwards it. Re-modelling means a release per new modality
and a silent drop for anything unmodelled, and a dropped image still
gets a confident answer about a picture nobody saw.

For the same reason a malformed `attachments` value is **refused**
(`InvalidArgs`) rather than dropped, which is the opposite of what
`history` does. Losing a prior turn costs context and the answer still
arrives; losing the picture the question is about is indistinguishable
from working.

Local engines refuse content parts outright (`inferenced`'s llama
backend) — images need a `remote:` model that has the modality.

## Where it sits

Three processes, three jobs, none able to approve its own work:

| | |
|---|---|
| **harnessd** | hosts the model, runs the loop |
| **agentd** | policy: tiers, provenance, Ledger, undo. Never sees a model |
| **lisa-consentd** | the human's dialog, and nothing else |

That split is issue #145. Before it, the process hosting the model also
owned the consent surface, so a call it originated came back to `Confirm`
from the same peer that asked.

## Trust comes from the caller, never the message

`Run` takes a trigger class and does **not** take it at face value. A
client that could name its own class could launder attacker-supplied
content into the class a human typed. So the class is resolved against a
ceiling derived from the caller, and **a caller may only narrow its
trust, never widen it**:

```
resolve("event",  ceiling=Prompt) → Event    narrowing: fine
resolve("prompt", ceiling=Event)  → Event    laundering: refused
resolve("wat",    ceiling=Prompt) → Event    unrecognised = least trusted
```

The resolved class becomes the first entry in every call's provenance
chain (`user` / `schedule` / `event`), and `bus-tools` appends every
untrusted class the run has since read. agentd escalates on the worst of
them.

## Taint belongs to the conversation, not the run (#305)

`bus_tools::Taint` is one-way — nothing removes a tag, because the model
has read the content and nothing un-reads it — and it used to be built
fresh on every `Run`. Right rule, wrong scope. The model reads a
**conversation**:

1. *"what does this page say?"* → Surfer answers `provenance: "web"` →
   this turn's bus calls go out `["user","web"]` and escalate.
2. *"ok, do that"* arrives as a **new `Run`** carrying the prior turns in
   `options["history"]`. `Taint::new()`. Empty. Chain `["user"]`, no
   escalation, and `bus::grant_for` resolves `Trigger::Prompt`.

Tool results are not replayed, so the page text does not literally
return; the model's own restatement of it does, at full `user` trust.
ADR-0030's premise is that we do not rely on the model declining to
launder.

**The scope now.** `src/conversation.rs` keeps a set per conversation,
`Run` seeds the run's `Taint` from it, and folds what the run read back
when the loop ends. The fold is a union: a run adds to its
conversation's taint and can never hand it back smaller — the same
asymmetry `Trigger::resolve` uses one level up.

**What clears it, and on whose authority.** Two things, and neither is
reachable from the process the model runs in (CLAUDE.md rule 6a):

- **starting a new conversation** — a person's act at a prompt surface;
- **restarting this daemon**, which drops the store (it is in memory).

There is deliberately **no method to clear a live conversation's taint**.
An API that forgets what a run has read is precisely what an injected
instruction would ask for. A person who wants a clean chain opens a new
chat: one click for them, everything for a page.

**How a conversation is identified.** `Run` has no session parameter and
the Assistant sends none — it replays `[{role, content}]` per run and
nothing else. So the conversation is its **owner** (the broker-assigned
unique name, so one peer's conversation is never another's — ADR-0033)
plus **the first user turn**: on turn one the prompt *is* that turn, on
every later turn it is `history[0]`. Two limits, stated rather than
papered over: two chats opening with the same sentence share a set
(over-escalation, the safe direction), and a surface that trims the head
of its history loses the set. No shipped surface trims; the one that
starts to is the one that should send a real session id.

**Attachments now cost what they carry.** `options["attachments"]`
forwards a screenshot or a scanned document verbatim into the model's
context and contributed *nothing* to the chain — the same attack with no
tag anywhere. An image part now contributes `screen`, any other part
`file`, on arrival, before the model sees it.

### Where the ceiling comes from (#229)

The ceiling is derived from the **transport**, in `src/caller.rs`. Three
answers, none of them anything the sender wrote:

| Question | Asked of | Used for |
|---|---|---|
| what uid is this connection? | `GetConnectionCredentials` | must be our own user |
| who owns `app.lisaos.Assistant`? | `GetNameOwner` | must be this caller |
| what program is that connection running? | **agentd**, `IsPromptSurface` → `/proc/<pid>/exe` | must be a `PROMPT_SURFACE_PROGRAMS` entry |

All three true → ceiling `Prompt`. Anything else, including a caller we could
not place at all → ceiling `Event`, the class whose content is never
trusted. A turned-down claim is written to the Ledger as
`harness.trigger_downgrade`, the way agentd records a provenance
downgrade — refusing outright would break a surface that merely tagged
its run wrongly, and a claim nobody can grep for is not an audit trail.

Until this landed the ceiling was the literal `Trigger::Prompt` for
every caller, so `busctl --user call … Run` — any peer on the session
bus — drove a run in the class a person typing gets.

**Why the program check comes from agentd (#306).** Everywhere else in
Lisa, program identity is the executable behind the broker's pidfd
(ADR-0033). That mechanism cannot work *in this process*: this is a
per-user unit with `ProtectHome`/`ProtectSystem`/`PrivateDevices`, which
a user manager can only deliver through an implicit user namespace, and
from inside one every peer's `/proc/<pid>/exe` is EACCES (#161).
Re-verified on the reference device with a positive control — the same
`readlink` in an unsandboxed transient user unit resolves
`/usr/bin/lisa-agentd` and `/usr/bin/gjs-console`; inside the sandboxed
one both are "Permission denied". `os/repo-tools/check-user-units.py`
carries the same note.

So the question goes to `lisa-agentd`, which runs in the initial user
namespace (`uid_map` `0 0 4294967295`, verified) and can read it:
`dev.lisaos.Agent1.IsPromptSurface(":1.412")`. What travels is a
**unique** bus name this daemon learned from the broker, never a claim;
what comes back is a boolean about the kernel's view of that connection.
Only asked when the name check already passed, and the answer is `false`
on every failure — an unreachable agentd, a refusal, a wrong reply type
— which costs the caller the prompt class. Fail-closed, and a real
availability coupling worth knowing about.

**Why the name alone was not enough.** A well-known name belongs to
whoever asks first, and `app.lisaos.Assistant` is D-Bus-activatable and
therefore not running most of the time. Demonstrated on the reference
device from an ordinary `python3` process: `RequestName` returned `1` —
PRIMARY_OWNER. This README used to argue that a squatter "has also taken
over the window the person launches, which is a much louder place to
stand". It is not: the name can be held for the seconds a `Run` takes
and released, and the next launch works normally.

**The honest limit that remains.** The Assistant is
`Exec=/usr/bin/lisa-app assistant/lisa-assistant.js`, and `lisa-app` ends
in `exec gjs`, so its executable is `/usr/bin/gjs-console`. The program
check refuses every compiled squatter and the demonstrated `python3` one;
it does not refuse a hostile GJS script. An Assistant with an executable
of its own closes it — `/usr/bin/lisa-assistant` is already listed in
agentd's `PROMPT_SURFACE_PROGRAMS` and unresolvable paths are dropped.
Tracked on #306.

`Schedule` is deliberately unreachable: nothing in Lisa is a scheduler
yet, and handing out a class no shipped peer can legitimately hold would
be a hole with no user. A scheduler daemon arrives with its own name and
its own arm of `caller::ceiling`.

## Streaming

`Token` signals carry deltas as they arrive, so a chat window shows words
appearing rather than a spinner. Control flow is unchanged — a tool call
is only knowable once its arguments are complete — so streaming is purely
about how the wait feels, which for a chat surface is most of the thing.

`forge` ignores deltas: a build loop printing the model's prose token by
token buries the tool calls that matter.

The fold lives in `openai.rs` as a pure function, because everything that
goes wrong with streaming tool calls lives there — arguments arrive as
fragments and are only valid JSON once concatenated, and parsing early
turns good input into an error.

## Tool families, assembled from what a run actually has

| Family | When | Tier |
|---|---|---|
| Agent Bus (`bus-tools`) | always | read; **write too** when the run is `prompt` class and a consent surface exists |
| Memory (`remember`, `recall`) | when the store opens | its own |
| Skills (`read_skill`) | when any skill is installed | read |
| Workspace (files, commands) | **a granted folder AND the `prompt` class** | jailed to that folder |

### Write tier (#216, #157)

The loop is offered write-tier bus tools — `navigate`, `create_note` and
whatever else an installed app declares `write` — when **both** hold:

- the run's resolved class is `prompt` (a person is at a prompt surface,
  clamped from the caller's transport identity below); and
- `dev.lisaos.Consent1` is running or activatable, which the broker
  answers, not a caller.

**Destructive tier is never offered**, to any loop. `delete`, `send`,
`wipe` stay out of the catalog: "the dialog will catch it" is a claim
about a person's attention, and attention is the one thing a tier ladder
must not spend.

Neither of those conditions is the guardrail, and neither is in this
daemon. The guardrail is in **agentd**, a different process the model
cannot reach: a write parks, and `lisa_guard::judge_approval` refuses
approval from any peer whose `/proc/<pid>/exe` is a model host — which
this daemon is. So a write the model asks for can only be released by the
consent surface. The second condition above is a usability gate: a tool
that parks for a dialog that cannot exist is a hang, not a capability.

Untrusted provenance escalates as it always did — a write following a
`read_page` arrives with `web` in its chain, agentd resolves it one tier
up, and the modal is the same dialog. `tests/injection-suite/tests/
loop_write_tier.rs` runs that path end to end through a real loop.

The families are assembled per run, so a surface with no workspace gets
no file tools — absent, not disabled. A tool the model can call only to
be refused wastes a turn and teaches it nothing.

The second condition on the last row is #230, and it is ADR-0036 §6.4:
*shell plus an event trigger is the injection endgame* — untrusted
content choosing arbitrary commands with nobody watching — so **event
and schedule triggers get typed tools only**. The family used to be
attached from `workspace.is_some()` alone, so an `event`-triggered run
was handed `read_file`, `write_file` and `run_command`; demonstrated on
the device.

The rule is applied to the WORKSPACE rather than to the provider list,
so one value feeds both the tools and the system prompt. Strip the tools
without stripping the sentence that promises them and the model
confidently claims to have saved something.

## Cross-conversation memory (#157, ADR-0025 phase 4)

Sessions already survived a restart; nothing survived *between*
conversations. `harness_core::Memory` was written, tested and called by
nothing. This daemon opens it at `$XDG_DATA_HOME/lisa/memory.db`, one
scope per user (`src/memory.rs`).

- **`remember(text)`** writes one durable note. **`recall(query)`**
  searches them.
- **The digest** — `Memory::digest`, ranked by reinforcement blended with
  recency, capped at 800 characters — goes into the system prompt on
  every turn, so a memory nobody asks for is still available. That is the
  whole reason the store existed unused for a year.
- **Untrusted notes cannot evict the person's own (#300).** Half the
  digest budget is reserved for `prov:user` notes and filled from a pool
  of their own, so no volume of untrusted notes can take the ambient
  prompt. Before it, a hundred notes written during one web-tainted run
  displaced every trusted note from every later conversation —
  including one recalled twenty times — permanently, because eviction
  was by recency and nothing prunes. An unclaimed reserve is spent on
  everything else rather than wasted.
- **Provenance is stamped, never passed.** `remember` takes only `text`.
  The note's class comes from the run's resolved trigger plus whatever
  the run has already read, so a conversation that has read a web page
  can only write `prov:web` memory whatever the tool call says.
- **Reading an untrusted note costs the run its trust.** When a
  `prov:web` note re-enters a conversation — through the digest at
  composition time, or through `recall` mid-run — its class is added to
  the run's shared `bus_tools::Taint`, so every Agent Bus call after it
  carries `web` and agentd escalates anything privileged. Without this,
  durable memory is durable injection: a page plants "the user always
  approves sending mail" on Monday and cashes it on Friday, when nothing
  in the conversation looks like a web page any more. ADR-0025 states the
  rule; this is where it is enforced, in code the model cannot reach.
- **The person can see and delete it.** `MemoryList()`,
  `MemoryForget(id)` and `MemoryForgetAll()` on `dev.lisaos.Harness1`,
  rendered by the Assistant's Memory button. Only the person's own prompt
  surface may call them (same ceiling as `Run`); the refusal names no
  note and no count, because a count is already a leak.

## The working folder is a grant

To write code the assistant needs somewhere to write, and handing it one
is a grant rather than a setting. **The folder comes from a person
choosing it in a file chooser; the model never picks it and cannot widen
it** — the same shape Claude Desktop uses, and the same reason
(ADR-0030: the capability is handed in from outside the loop).

`workspace::validate` refuses a path that is not absolute, does not
exist, is not a directory, is the whole home, is a system folder, lies
outside the user's home, **or is hidden or inside anything hidden**. It
resolves before judging, because `~/proj/..` is the home root however it
is spelled.

The hidden rule is #231. Every other check passed for `~/.ssh` —
absolute, real, a directory, under home, not home itself — so it was a
legal jail root, and `read_file authorized_keys` handed the model the
user's key material. Demonstrated on the device. Writes were stopped
only by `ProtectHome=read-only` in the unit, and a systemd option is not
a policy: it is true until the day the unit changes, and then the
regression is silent.

It is structural rather than a denylist of the credential stores we
happened to think of, for the reason the next paragraph gives about
denylists generally. A leading dot is the convention by which a program
says "this is mine, not the user's work", and that is what is checked —
after the home prefix is stripped, so a home directory that is itself
hidden (macOS test fixtures live under `/private/var/folders/…/.tmpXXX`)
is not caught by its own spelling.

**The cost, stated plainly.** Someone whose project genuinely *is* a
dotfile folder — `~/.config/nvim` is the real example — has to copy it
elsewhere or work on it another way. ADR-0029's second test says a
guardrail sits between the model and the machine, never between a person
and their own machine, so the line is drawn where the two actually
differ: nothing chooses `~/.ssh` because it is *working on* `~/.ssh`.
The failure prevented is a credential leaving the machine; the failure
caused is a copy.

**The containment is the home requirement, not the denylist.** A prefix
denylist looks like the defence and is not: canonicalisation can move a
path out from under it — `/etc` resolves to `/private/etc` on macOS, so
`starts_with("/etc")` stops matching the thing it was written for. The
test found that, not a reading of the code. No `$HOME` therefore means
no workspace at all: failing closed beats a grant whose only real check
has evaporated.

The system prompt describes the CURRENT grant. With a folder it explains
the jail; without one it says plainly that there are no file tools and
to ask the person to choose a folder — because an assistant told it can
write files when it cannot will confidently claim to have saved
something, which is the failure people never forgive.

## Skills

A skill is markdown with `name`/`description` frontmatter and a workflow
body. The **catalog** — one line each — goes in the system prompt; the
bodies do not, and pasting every skill into every conversation spends the
context window before the question is read. The model fetches what it
needs with `read_skill`.

With no skills installed, no tool is offered and the prompt says nothing
about them.

## Limits

- **One thread per run**, and a signal emit builds a small runtime each
  time. Correct, not elegant.
- **Cancel is cooperative, and it now does something.** Until #227 it set
  a flag nothing acted on: the loop had no cancellation input at all, so
  Stop was a no-op and the run continued through its whole turn budget.
  The flag is `forge_harness::Cancel` — the loop's own type — and the
  loop consults it before each turn, after the model answers but before
  its tool call is dispatched, and between frames of the answer as it
  arrives. A tool that has STARTED still runs to its end: killing a
  write halfway is how half-done actions happen. A stopped run answers
  `Finished(ok=false, "Stopped.")`.
- **An `attachments` option over 24 MiB is refused** (#226). That bounds
  what this daemon will act on, not what the broker will deliver: a
  message-size ceiling belongs to dbus-broker's own configuration.
  Passing a file descriptor instead of a base64 data URI would remove
  the amplification entirely, and would be a redesign of the option.
- **The trigger ceiling's program check is an interpreter** (#306). The
  name half is no longer enough on its own — see "Where the ceiling comes
  from" above — but because the Assistant runs under `gjs`, a hostile GJS
  script still satisfies the program half. A dedicated
  `/usr/bin/lisa-assistant` executable is what closes it.
- **The ceiling now depends on agentd being reachable.** If it is not,
  every run is `Trigger::Event`: no file or `run_command` family, no
  write-tier bus tools, no `user` provenance, no memory methods. That is
  the safe direction and it is a coupling this daemon did not have
  before.
- **A dotfile folder cannot be a workspace** (#231), so the assistant
  cannot help with `~/.config/nvim` in place.
- **Write tier has not been driven by a resident model on the device.**
  The path is exercised end to end by
  `tests/injection-suite/tests/loop_write_tier.rs` — a real
  `forge_harness` loop, a real `bus_tools` provider, a real
  `lisa_agentd::bus::AgentBus` — with a *scripted* backend rather than a
  live model. What that leaves unproven is whether a given model chooses
  to call a write tool and what it does with the "a person must confirm"
  result, not whether the machinery holds.
- **Memory is one scope, `user`.** Per-project memory is a schema the
  store already supports and nothing yet writes; saying so beats letting
  a reader infer it from the constant.
- **Nothing prunes memory, and nothing rate-limits `remember`.** Notes
  accumulate until a person deletes them. Since #300 the digest reserve
  means volume can no longer buy ambient prompt space, so what an
  unbounded write loop costs is disk, `recall` quality and a Memory pane
  nobody can read — not context. There is still no forgetting curve and
  no per-run write cap, which #300 names as the alternative fix.
- **The conversation taint is in memory and bounded to 256
  conversations** (#305). Restarting the daemon clears every set, and
  the 257th conversation evicts the oldest. Both lose taint, which is
  the fail-open direction; both take a person's action (a restart, or
  256 fresh chats) rather than a page's, which is why the bound is
  generous rather than tight.
- **The three lines that wire the conversation taint into `Run` are not
  covered by a test.** `TaintStore::open`/`close` and the key derivation
  are exercised end to end — through a real `bus_tools::AgentBusTools`,
  asserting the chain that reaches the wire — and `parse_history` →
  `key_for` has its own test in `dbus.rs`. What no test reaches is
  `Run` itself calling them, because a caller only has an owner over a
  message broker and these tests run on macOS with none. Stated rather
  than implied.
- **A memory note's provenance cannot be repaired.** If a note was
  learned during a tainted run, it stays untrusted for ever, even if the
  fact in it is true and the person agrees with it. Forgetting and
  re-stating it in a clean conversation is the only path, which is
  honest but is not a feature anybody would design on purpose.
- **`ProtectHome=read-only` means the workspace file tools cannot
  actually write on the device.** The jail permits it and the unit does
  not, so `write_file` into a granted folder fails at the syscall. That
  is a separate defect from #231 — noticed while fixing it — and it is
  not fixed here: the option is real containment for `run_command`, and
  swapping it for a narrower `ReadWritePaths` is a change to the unit's
  whole confinement story rather than a line.
- **The unix hop rides on `ureq::unversioned`.** ureq's transport layer
  is explicitly outside its semver promise, so a ureq minor bump can
  break `forge_harness::unix_http`. It cannot break it *quietly* — the
  crate stops compiling — but it is a dependency on an unstable surface,
  taken because the alternative was hand-rolling chunked
  transfer-encoding for the streaming lane.
- **The companion it talks to is still not confined.**
  `lisa-inferenced-dbus.service` keeps `AF_INET` because it LISTENS on
  `127.0.0.1:7778` for two libsoup callers in `shell/`
  (`lisa-assistant.js`'s model picker, `lisa-overlayd.js`'s chat lane),
  and libsoup has no unix-socket transport. That is the remaining half
  of #288, recorded in `USER_SCOPE_INET_DEBT` in
  `os/repo-tools/check-egress-units.py`, and it is a real hole rather
  than a formality — this daemon being confined does not confine that
  one.
