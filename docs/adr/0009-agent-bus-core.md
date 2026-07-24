# ADR-0008: Agent Bus core — D-Bus surface, tier enforcement at the bus, staged MCP transport

- **Status:** accepted
- **Date:** 2026-07-22

## Context

M5 (PLAN §5.4) is the Agent Bus: apps are MCP servers, `lisa-agentd` is
the system MCP host, and confirmation tiers + an undo journal are
enforced for every tool call, with the injection suite (§5.10) gating
merges. The section names `libs/mcp-bus` ("Rust; vendored MCP SDK") and
a per-app unix-socket transport with D-Bus-activation spawn-on-demand.
That transport is a large surface (an MCP client, process activation,
socket lifecycle) and it needs a running desktop to be meaningful. The
milestone is explicitly big; PLAN §12 says milestones are serialized and
anything outside a spec's Acceptance block is backlog, so the first PR
must deliver a *coherent enforceable core*, not the whole section.

Two design questions had to be settled before writing code:

1. **Where is the tier/provenance policy enforced?** PLAN §5.4 says
   "enforced at the bus, not by app goodwill" and rule 6 says untrusted
   provenance escalates. If enforcement lived in the model prompt (the
   Appendix C guardrail), a jailbroken model would defeat it — the exact
   thing the injection suite tests.
2. **How is the bus reached, and by whom?** §5.4 requires scriptability
   ("everything on the bus is reachable from the terminal") and PR #2's
   overlay backend (`dev.lisaos.Overlay1`) was shaped to become an Agent
   Bus client at M5. Both point at a D-Bus surface, but §5.4 does not
   sketch one (Appendix A only sketches `dev.lisaos.Inference1`).

## Decision

**Enforce tiers and provenance escalation in the bus state machine
(`daemons/agentd/src/bus.rs`), independent of the model.** The Appendix C
guardrail prompt (`daemons/agentd/prompts/system-policy.md`) is the
prompt half of defense-in-depth, but the load-bearing guarantee — *no
privileged dispatch without confirmation* — is a code path the model
cannot reach around. Only a `read`-tier tool with a fully trusted
(all-`user`) trigger chain executes silently; every other call parks for
`chip`/`modal` confirmation. An **empty chain is treated as untrusted
(fail closed)**: absence of provenance escalates rather than defaulting
to trusted.

**Expose the bus as `dev.lisaos.Agent1` on the session bus** with
`ListTools`, `Discover`, `RequestCall`, `Confirm`, `Undo`, and a
`ConfirmationRequested` signal. Rich payloads (tool lists, call specs,
undo reports) cross as JSON strings, matching how `dev.lisaos.Overlay1`
already ships JSON in `meta_json`/`status` and keeping the surface
`busctl`- and script-friendly per §5.4. `RequestCall` takes the trigger
chain as a `provenance` array in its options `a{sv}`; omitting it means
unknown origin, which escalates.

**Stage the MCP wire transport behind a `Dispatcher` trait.** This PR
ships `NullDispatcher` (production, until the transport lands) and a
recording dispatcher (tests). The per-app unix-socket MCP client, the
`libs/mcp-bus` crate extraction, D-Bus-activation spawn-on-demand, and
btrfs-snapshot compensation for file ops are deferred to the next M5
slice — the enforcement core, registry, discovery, journal, and D-Bus
surface are complete and tested without them.

**The undo journal is separate from the Ledger.** The Ledger stays the
append-only audit trail (every `tool.call`/`confirm`/`complete`/`deny`/
`undo` lands there first — no ledger entry, no action). The journal
(`agent-journal.db`, beside the ledger) is mutable working state whose
entries move active → undone/skipped, so `lisa undo` can pop the action
stack. Compensations come from the manifest `undo` declaration
(Appendix B), resolved against the executed call's `$input`/`$result`.

**Injection suite is bus-layer first.** The 500-attempt corpus and its
0-unconfirmed-privileged-call gate run against the real `AgentBus` with a
recording dispatcher — host-independent, no model, no desktop. The
model-in-the-loop layer (feed each payload through the system prompt +
resident model, assert the emitted plan) is deferred with the transport.

## Consequences

- The M5 safety guarantee is testable on macOS/Linux today and is true
  even against a fully compromised model — the property the §5.10 gate
  actually cares about. The first corpus slice is 150 attempts (10
  payloads × 5 vectors × 3 targets); reaching 500+ is adding payloads,
  not reworking the harness.
- The overlay backend and future app tools have a stable interface to
  build against now (`dev.lisaos.Agent1`, JSON payloads, provenance array)
  even though no first-party app ships MCP tools yet and no live tool
  executes (NullDispatcher). PR #2's overlay swaps its direct
  `dev.lisaos.Inference1` calls for `RequestCall` when it becomes an Agent
  Bus client.
- Deferring the MCP transport means the demo flow in the §5.4 Acceptance
  block (Calendar+Mail discovery → 2-step plan → tiers → Ledger trace →
  `lisa undo`) is not yet end-to-end runnable; it is proven in parts
  (discovery, tiers, journal, undo) at the bus layer and completes when
  the transport + first-party tools land.
- `libs/mcp-bus` does not exist yet; the `Dispatcher` trait is the seam
  it will slot into. If the vendored MCP SDK choice changes, only the
  dispatcher implementation moves — the enforcement core does not.
