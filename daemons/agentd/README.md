# lisa-agentd — system agent & MCP host

Spec: docs/PLAN.md §5.4. Milestone: M5. Design decisions: ADR-0009.

Registry and client of app MCP servers; executes tool calls under
**bus-enforced** confirmation tiers (read/write/destructive) with
provenance escalation and an undo journal. The guardrail prompt
(`libs/harness-core/prompts/system-policy.md`, compiled in as
`harness_core::policy` — agentd hosts no model and builds no prompt,
which is why it never belonged here, issue #58) mirrors Appendix C; the injection suite
(`tests/injection-suite`) gates merges.

## What this crate implements (M5 first slice)

- **`manifest`** — Appendix B manifest parsing + strict validation
  (versions, reverse-DNS app ids, unix transport, tool-name rules,
  object input schemas, undo declarations point at real same-manifest
  tools and use well-formed `$input`/`$result` maps), plus a minimal
  structural args validator (types, `required`, closed objects).
- **`registry`** — installed-manifest registry (one broken manifest is
  skipped, never fatal) and tool discovery by token-overlap ranking
  ("what can handle 'add a task'?").
- **`tier`** — the confirmation-tier policy: read → silent, write →
  chip, destructive → modal; any untrusted provenance in the trigger
  chain escalates one tier, and an **empty chain fails closed** (unknown
  origin is untrusted). Only `user` provenance is trusted.
- **`bus`** — the call state machine: request → **action guard** →
  tier resolution → silent execute *or* park for confirmation →
  confirm/deny → execute.
  Every path is ledgered (`tool.call`/`confirm`/`complete`/`deny`/
  `undo`) *before* it happens (no ledger entry, no action). Executed
  privileged calls are journaled with their resolved compensation.
- **`journal`** — the undo journal (`agent-journal.db`, beside the
  ledger): mutable working state (active → undone/skipped) so `lisa
  undo` reverts **the caller's own** last agent action via the
  manifest-declared inverse. Each row records the transport-assigned
  `owner`, and `last_active` is scoped to it (#94): undo dispatches a
  compensation that is frequently destructive-tier, so an unscoped query
  let any session peer revert any other peer's action. Ownership rather
  than a fresh confirmation is deliberate — undo reverts an action this
  peer *made*, and having made it is the authority; re-asking would make
  undo unusable while adding nothing. Rows written before the `owner`
  column existed have NULL and are undoable by nobody, which is the
  fail-closed direction
  call.
- **`dbus`** — the `dev.lisaos.Agent1` session-bus surface (below).

## D-Bus surface: `dev.lisaos.Agent1`

JSON payloads cross as strings (script/`busctl`-friendly, matching
`dev.lisaos.Overlay1`):

```
ListTools() → (s tools_json)              # [{app_id,name,tier,description,undoable}]
Discover(s query) → (s tools_json)
RequestCall(s app_id, s tool, s args_json, a{sv} options)
    → (t call_id, s disposition, s detail_json)
    options: "actor" (s), "provenance" (as — the trigger chain;
             omitted/empty = unknown = escalates)
    disposition: "executed" | "failed" | "confirm-chip" |
                 "confirm-modal" | "denied" | "refused"
Confirm(t call_id, b approve) → (s status, s detail_json)
Undo() → (s report_json)
signal ConfirmationRequested(t call_id, s spec_json)
signal RefusalReported(t call_id, s report_json)
```

Read-tier calls with a fully trusted (all-`user`) chain execute
immediately; everything else parks and emits `ConfirmationRequested`
(answer via `Confirm`). The overlay backend (`dev.lisaos.Overlay1`, §5.7.1)
becomes a client of this interface, swapping its direct
`dev.lisaos.Inference1` calls for `RequestCall` when it turns tool calls
into agent actions.

### The refused disposition (#251, #252)

Before any tier is resolved, the call is judged by
`lisa_guard::judge_action` against `(tool, arguments, grant)`. A refused
call **is never parked** — there is no pending entry, so there is no id
any dialog could approve. `Confirm` on it answers `UnknownCall`, exactly
as it would for a call that was never made.

That ordering is the guardrail. It is not a dialog with a scarier title;
it is the absence of the state a dialog acts on (ADR-0029, CLAUDE.md 6a).

A refusal is still **reported**, on its own signal: silent refusal hides
an attack, and if hostile content just caused the model to attempt
`rm -rf /`, that is precisely the event the owner needs to see. The
report carries the rule, a reason, the provenance and an occurrence
count — and deliberately **no arguments and no command**, because a
copy-to-clipboard or a "run this for me" affordance is the Allow button
rebuilt with extra steps.

`refused` is a distinct disposition from `denied` on purpose: a denial is
a person saying no this time, a refusal is an action that will never be
available to an agent. A caller that cannot tell them apart retries the
second one forever.

Every refusal appends a `tool.refuse` Ledger entry with
`status: "hard-no" | "out-of-scope"`, filed under the **caller** as the
transport names it, carrying `occurrence` — so one refusal reads as an
event and the same actor refused three times reads as an attack in
progress, without anyone counting rows by eye (#217).

The grant the verdict is measured against is built from outside the
model's reach — `$HOME` and the uid of the process, and the trigger class
derived from the verified chain. Nothing over D-Bus can widen it;
`with_grant` is a constructor, not a method call. `workspace` and
`scratch` are not yet wired from harnessd, so today every path is judged
against the home ladder alone.

### Who may answer a parked call

Authority comes from the broker, never from the message (ADR-0033).
agentd asks the bus who owns `dev.lisaos.Consent1` — the desktop consent
surface, `shell/consent` — and compares that to the transport-assigned
sender:

| answering peer | withdraw (`false`) | approve a chip | approve a modal |
|---|---|---|---|
| the requester | yes, always | yes — the app's own inline affordance | **no** |
| the consent surface (a different peer) | yes | yes | yes |
| anyone else | no (`NotYours`) | no | no |

The requester may never approve its own destructive call, whatever the
consent name says and whether or not anything owns it. The single
exemption is a point-to-point connection, where there is no broker to
ask and requester and answerer are the same peer by construction; the
daemon's own tests use that transport and `main.rs` never builds one.

Before a modal parks, agentd **starts** the consent surface if nothing
owns the name (`StartServiceByName`, which — unlike `GetNameOwner` —
activates, and returns only once the name is owned, so the signal cannot
outrun its listener). Until #244 nothing ever made that call: the surface
shipped activatable and never once ran, every confirmation resolved to
"no surface exists", and that resolved in turn to "so the requester
answers its own call".

A refused approval is recorded as `tool.deny`/`refused`, once per parked
call, naming the peer and why it was not an independent surface.

## App manifests

Manifests are Appendix B JSON files loaded (later dir wins on app-id
clash) from, in order: `/usr/share/lisa/manifests`, then
`$XDG_DATA_HOME/lisa/manifests` (or `~/.local/share/lisa/manifests`).
`LISA_MANIFEST_DIRS` (colon-separated) overrides both for testing.

## Deferred to later M5 slices (ADR-0009)

The MCP wire transport (per-app unix socket + D-Bus-activation
spawn-on-demand) is behind the `bus::Dispatcher` trait; production wires
`NullDispatcher` (every dispatch fails cleanly and is ledgered) until it
lands. Also deferred: `libs/mcp-bus` extraction, `lisa tools/call/undo`
CLI verbs, btrfs-snapshot compensation for file ops, first-party app
tools, and the model-in-the-loop injection layer. Because no transport
is wired, the §5.4 Acceptance demo flow is proven in parts (discovery,
tiers, journal, undo) at the bus layer, not yet end-to-end.

## Egress

No network — ever (CLAUDE.md rule 5). The hardened systemd unit enforces
it on the image; no dependency here may add a network path.
