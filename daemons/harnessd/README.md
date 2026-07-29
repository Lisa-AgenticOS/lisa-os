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
    options: "model" (s), "url" (s), "trigger" (s: prompt|schedule|event)
Cancel(t run_id)
signal Tool(t run_id, s name, s detail)
signal Token(t run_id, s delta)
signal Finished(t run_id, b ok, s summary)
```

Shaped like `Overlay1`'s Ask/Token/Finished deliberately: the Assistant
window already renders that vocabulary, so adopting the harness is a
change of destination, not a rewrite.

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

## Limits

- **One thread per run**, and a signal emit builds a small runtime each
  time. Correct, not elegant.
- **Cancel is cooperative** — a turn already in flight finishes. Killing
  a tool call halfway is how half-done actions happen.
- **The trigger ceiling is hardcoded to `Prompt`** because every caller
  today is a desktop surface. It must come from `lisa-peer` identity
  before any non-desktop client exists.
