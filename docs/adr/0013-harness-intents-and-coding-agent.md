# ADR-0013: The Lisa harness — Siri-style intents + a Claude-Code-level coding agent, on the existing substrate

- **Status:** proposed
- **Date:** 2026-07-24

## Context

The directive: make Lisa's assistant a **Claude-Code / Pi-level agentic
worker** *and* a **Siri-with-App-Intents** system — natural language →
structured, confirmed actions — informed by the **Hermes** tool-calling
convention and by `flakerimi/harness` (a mature Go agent harness whose
pillars we want: Soul/identities, Memory, Skills, Sessions, Crons, Hands,
Background tasks, Self-improvement).

A survey of the repo (2026-07-24) found the substrate is already most of
the way there, and the missing pieces are **disjoint by crate**:

- `daemons/agentd` — the Agent Bus — is built: confirmation **tiers**,
  **provenance escalation** (untrusted input escalates one tier,
  fail-closed), an **undo journal**, **Ledger** enforcement, and a
  merge-gating **injection suite (600 attempts, 0 unconfirmed privileged
  calls)**. The one hole: its `Dispatcher` is a `NullDispatcher` — nothing
  actually executes, because there is no MCP transport yet.
- `libs/liblisa` has **guided generation** (`json_schema_to_gbnf`) and a
  `Task` abstraction — the primitive for turning an utterance into a
  *guaranteed-valid* typed value. But there is **no NL→intent router**.
- `libs/forge-harness` is a single-shot whole-file + `dart analyze` loop
  behind a path-traversal **Jail** — a coding-agent *seed*, not a
  multi-tool agent.

## Decision

Build the harness **in Rust, on this substrate** — not by vendoring the
Go harness. `flakerimi/harness` is the **design template** (its pillars
become the phase-2 roadmap); the engine is Lisa's, so intents inherit the
bus's trust guarantees for free.

**1. Intents = guided generation → the Agent Bus.** A new NL→intent
router (`liblisa`) builds an output JSON Schema from the catalog of
available tools (`{intent: "<app_id>::<tool>" | "none", confidence,
args}`) and drives it with a Hermes-style system prompt. Because the
schema is grammar-constrained, the local model **cannot emit an invalid
intent** — strictly stronger than Hermes prompt-only tool-calling on
small models. The router's output maps to `Agent1.RequestCall(app_id,
tool, args, provenance)`, so every intent passes through tiers, undo,
Ledger, and the injection gate. Siri "App Intents", but the intent layer
is native MCP (PLAN §5.4), both directions.

**2. Execution = an MCP dispatcher.** `libs/mcp-bus` implements a per-app
unix-socket MCP client and an `McpDispatcher` that satisfies agentd's
`Dispatcher` shape, replacing `NullDispatcher`. This is the single change
that turns the whole bus from "decides correctly, executes nothing" into
a working action layer.

**3. Coding agent = forge-harness, generalized.** Extend the seed into a
multi-turn, multi-tool loop (read/list/grep/edit-diff/run/test) with
every file op mediated by the **existing Jail**, a parameterized verifier
(not just `dart analyze`), and a tool-calling conversation. The jail stays
the security boundary even against a hostile model.

**4. Pillars later (phase 2).** Memory / Skills / Sessions / Crons /
Background tasks / Self-improvement — ported from `flakerimi/harness` into
a Lisa `harness` component that composes the intent router (assist),
forge-harness (code), and the bus (act) behind one surface (the overlay,
which §5.7.1 already designed to swap `Inference1`→`Agent1` unchanged).

**Delivery is parallel + isolated.** Phase 1's three pieces are built by
separate agents in separate git worktrees (one crate each: forge-harness,
liblisa, mcp-bus), each keeping its crate green, then reviewed and merged
to `main` by the orchestrator. No shared files → no collisions.

## Consequences

- Every assistant action is trust-checked by construction; there is no
  second, weaker path around the bus.
- Guided-gen intents need `json_schema_to_gbnf` to grow `oneOf` for
  per-intent argument schemas; until then the router uses a flat
  `{intent, args}` shape (args validated structurally at the bus).
- A working `McpDispatcher` unblocks the PLAN §5.4 acceptance demo
  (utterance → multi-step confirmed tool calls) end to end.
- The Go harness is not a dependency; its ideas are, so we keep one
  toolchain (Rust) and the local-first/no-egress guarantees.

## Alternatives considered

- **Vendor `flakerimi/harness` (Go) as the engine.** Fastest to the
  pillars, but splits the toolchain, and every intent would have to
  re-cross into the Rust bus for trust enforcement — reimplementing the
  seam we already have. Rejected as the *engine*; kept as the *template*.
- **Prompt-only Hermes tool-calling.** Simpler, but unreliable on the
  small local models Lisa targets; guided generation already gives
  1000/1000 valid, so we use the schema as the calling convention.
