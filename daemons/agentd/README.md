# lisa-agentd — system agent & MCP host

Spec: docs/PLAN.md §5.4. Milestone: M5. Design decisions: ADR-0009.

Registry and client of app MCP servers; executes tool calls under
**bus-enforced** confirmation tiers (read/write/destructive) with
provenance escalation and an undo journal. The guardrail prompt
(`prompts/system-policy.md`) mirrors Appendix C; the injection suite
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
- **`bus`** — the call state machine: request → tier resolution →
  silent execute *or* park for confirmation → confirm/deny → execute.
  Every path is ledgered (`tool.call`/`confirm`/`complete`/`deny`/
  `undo`) *before* it happens (no ledger entry, no action). Executed
  privileged calls are journaled with their resolved compensation.
- **`journal`** — the undo journal (`agent-journal.db`, beside the
  ledger): mutable working state (active → undone/skipped) so `lisa
  undo` reverts the last agent action via the manifest-declared inverse
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
                 "confirm-modal" | "denied"
Confirm(t call_id, b approve) → (s status, s detail_json)
Undo() → (s report_json)
signal ConfirmationRequested(t call_id, s spec_json)
```

Read-tier calls with a fully trusted (all-`user`) chain execute
immediately; everything else parks and emits `ConfirmationRequested`
(answer via `Confirm`). The overlay backend (`dev.lisaos.Overlay1`, §5.7.1)
becomes a client of this interface, swapping its direct
`dev.lisaos.Inference1` calls for `RequestCall` when it turns tool calls
into agent actions.

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
