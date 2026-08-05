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
chain (`user` / `schedule` / `event`), and `bus-tools` appends `web`
once anything web-tagged has been read. agentd escalates on the worst of
them.

### Where the ceiling comes from (#229)

The ceiling is derived from the **transport**, in `src/caller.rs`. Two
answers, both from the message broker and neither of them anything the
sender wrote:

| Question | Asked of | Used for |
|---|---|---|
| what uid is this connection? | `GetConnectionCredentials` | must be our own user |
| who owns `app.lisaos.Assistant`? | `GetNameOwner` | must be this caller |

Both true → ceiling `Prompt`. Anything else, including a caller we could
not place at all → ceiling `Event`, the class whose content is never
trusted. A turned-down claim is written to the Ledger as
`harness.trigger_downgrade`, the way agentd records a provenance
downgrade — refusing outright would break a surface that merely tagged
its run wrongly, and a claim nobody can grep for is not an audit trail.

Until this landed the ceiling was the literal `Trigger::Prompt` for
every caller, so `busctl --user call … Run` — any peer on the session
bus — drove a run in the class a person typing gets.

**Why a bus name and not `/proc/<pid>/exe`.** Everywhere else in Lisa,
program identity is the executable behind the broker's pidfd
(ADR-0033). That mechanism cannot work *here*: this is a per-user unit
with `ProtectHome`/`ProtectSystem`/`PrivateDevices`, which a user
manager can only deliver through an implicit user namespace, and from
inside one every peer's `/proc/<pid>/exe` is EACCES (#161). An exe check
in this daemon would be a check that silently never matches. Verified on
the reference machine: harnessd's `uid_map` is `1000 1000 1`, and
readlink of a peer's `exe` succeeds outside the namespace and fails
inside it. `os/repo-tools/check-user-units.py` carries the same note.

**The honest limit.** A well-known name belongs to whoever asks first,
so a peer that grabs `app.lisaos.Assistant` while the Assistant is
closed inherits its ceiling. That is smaller than the hole it replaces —
before, no peer had to do anything at all — and a peer holding the
Assistant's name has also taken over the window the person launches,
which is a much louder place to stand.

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
- **The trigger ceiling is a bus name, not an executable.** See "Where
  the ceiling comes from" above: it is the strongest identity available
  to a daemon that must keep its mount sandbox (#161), and a peer that
  takes `app.lisaos.Assistant` while the Assistant is closed inherits
  its ceiling.
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
- **Nothing prunes memory.** Notes accumulate until a person deletes
  them. The digest is bounded, so an oversized store costs disk and
  `recall` quality rather than context — but there is no forgetting
  curve, and a store that only grows will eventually need one.
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
