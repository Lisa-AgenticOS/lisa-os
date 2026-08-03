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

Today every session peer is a desktop surface, so the ceiling is
`Prompt`. The enforcement point exists now so that when cron and mail
arrive they get a lower ceiling and nothing else has to change.

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
| Agent Bus (`bus-tools`) | always | read-tier only until #145 lands |
| Skills (`read_skill`) | when any skill is installed | read |
| Workspace (files, commands) | **only with a granted folder** | jailed to that folder |

The families are assembled per run, so a surface with no workspace gets
no file tools — absent, not disabled. A tool the model can call only to
be refused wastes a turn and teaches it nothing.

## The working folder is a grant

To write code the assistant needs somewhere to write, and handing it one
is a grant rather than a setting. **The folder comes from a person
choosing it in a file chooser; the model never picks it and cannot widen
it** — the same shape Claude Desktop uses, and the same reason
(ADR-0030: the capability is handed in from outside the loop).

`workspace::validate` refuses a path that is not absolute, does not
exist, is not a directory, is the whole home, is a system folder, or
lies outside the user's home. It resolves before judging, because
`~/proj/..` is the home root however it is spelled.

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
- **Cancel is cooperative** — a turn already in flight finishes. Killing
  a tool call halfway is how half-done actions happen.
- **The trigger ceiling is hardcoded to `Prompt`** because every caller
  today is a desktop surface. It must come from `lisa-peer` identity
  before any non-desktop client exists.
