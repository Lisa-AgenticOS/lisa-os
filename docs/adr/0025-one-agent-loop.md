# ADR-0025: One agent loop — the Lisa harness

- Status: accepted (design; phased implementation)
- Date: 2026-07-26
- Relates: ADR-0013 (harness program, the pillar model), ADR-0009 (Agent
  Bus), ADR-0015 (Assistant), PLAN §5.4 / §5.12.1, issue #25
- Supersedes nothing; it unifies what ADR-0013 started

## Context

Lisa has three agent entry points that share almost nothing:

| verb | what it is | tools | memory | session | verifier |
|---|---|---|---|---|---|
| `lisa ask` | one-shot chat, streams tokens | none | none | none | none |
| `lisa do` | NL → **one** Agent Bus tool call | bus catalog | none | none | consent tier |
| `lisa forge` | multi-turn coding agent | 7 jailed workspace tools | none | none | `dart`/`flutter analyze` |

Meanwhile `libs/harness-core` — Sessions, Memory, Skills, a KV store, Turn —
is written, tested, and **wired into nothing**. The pillars ADR-0013 named
(Soul, Memory, Skills, Sessions, Crons, Hands, Tasks) exist as a library
nobody calls.

The tools the field actually wants (Claude Code, Codex, Pi, and the
flakerimi/harness design this program templates from) are not three
programs. They are **one loop**: a conversation that can call tools, where
"edit a file", "run a command", "create a note" and "answer in words" are
all just tools, and where sessions, memory and skills are ambient rather
than per-verb features. Writing code is not a different mode; it is the
same loop with filesystem tools available.

Splitting by verb also produces the bugs we keep hitting: `lisa do` could
not use Claude at all until tool-calling landed (the grammar router is
local-only), while `forge` already had a working tool-calling loop three
files away. Two implementations of "route an utterance to a tool" that
learn nothing from each other.

## Decision

**One loop. Many tool families. Surfaces are thin frontends.**

```
        lisa agent │ lisa do │ lisa forge │ Assistant │ overlay │ Ambient
                   └────────────┬───────────────────────────────┘
                        harness::Loop  (libs/harness-core)
        ┌───────────────┬───────┴────────┬──────────────┬─────────────┐
     Sessions        Memory           Skills          Tools        Model
   (resumable      (durable        (instructions   (three         (local
    transcript)     notes)          on demand)      families)      or remote)
```

### The loop

One turn: assemble context (session transcript + relevant memory + loaded
skills) → ask the model with the tool catalog → execute the tool call →
append the result → repeat until the model answers without a tool call, a
verifier passes, or the budget is spent. This is `forge-harness`'s existing
agent loop, generalised: its `Verifier` becomes optional, its jailed tools
become one tool family among several.

### Three tool families, one catalog

1. **Workspace tools** (`forge-harness::tools`) — read/write/edit/grep/
   list/run in a jail. Present only when the loop has a workspace.
2. **Agent Bus tools** (`dev.lisaos.Agent1`) — app capabilities with tiers,
   provenance, consent chips, undo, and the Ledger. Always present.
3. **Harness tools** — `remember`/`recall` (Memory), `load_skill` (Skills),
   `new_task` (Tasks). Small, and the pillars stop being inert.

All three are presented to the model in one catalog, by the same wire
format. **Capability decides the routing, not the verb**: models exposing
native tool calling get `tools`; local models get the GBNF two-stage
router, whose output is *guaranteed* well-formed — which small models need
(cli/lisa/src/agent.rs, already live).

### Sessions, Memory, Skills

- **Sessions** — every loop run belongs to a session, persisted through
  `harness-core::SessionStore` over Context1 keys, so any surface can
  resume any conversation. The Assistant already writes this layout
  (issue #25); the CLI and overlay adopt the same store, which is the
  whole point of the key layout being shared.
- **Memory** — durable notes with provenance, offered as `remember` and
  `recall`. Provenance is load-bearing: a memory written from untrusted
  content may never silently authorise a privileged tool call (PLAN §5.10,
  Appendix C).
- **Skills** — `SKILL.md` files with `name`/`description` frontmatter. Only
  the one-line descriptions stay in context; the body loads when the model
  calls `load_skill`. This is how a repo teaches Lisa a workflow without
  paying for it every turn, and it is the mechanism by which **Lisa learns
  to build Flutter apps for itself**: the lisa_ui conventions, the
  scaffold, and the verifier recipe are a skill, not hardcoded Rust.

### Safety is unchanged, and now uniform

Bus tools keep tiers + consent + undo. Workspace tools keep the jail.
Every model call is ledgered before it happens (the existing invariant).
The difference is that these rules now apply to *one* loop instead of
being re-derived per verb.

## Phases

1. **Loop extraction** — generalise `forge-harness`'s agent loop into
   `harness-core::Loop` with a pluggable tool catalog; `lisa forge` becomes
   its first caller with the workspace family + Dart verifier. No behaviour
   change, all existing tests must pass unmodified.
2. **Bus family + `lisa agent`** — the bus catalog as a tool family; a
   conversational CLI entry point. `lisa do` becomes "one turn, bus family
   only" — the same loop with a turn budget of 1.
3. **Sessions everywhere** — CLI and overlay adopt the SessionStore the
   Assistant already writes; `--resume` and `--session`.
4. **Memory + Skills tools** — `remember`/`recall`/`load_skill`, and the
   first shipped skill: *building a lisa_ui Flutter app*.
5. **Tasks + Crons** — background and scheduled runs, ledgered, surfaced in
   the Assistant. (Deliberately last: a scheduler that can call tools
   unattended needs the consent story settled first.)

## What was rejected

- **Keep three verbs, share a library.** Tried implicitly; it produced two
  routers, and only one of them worked with cloud models. Shared code under
  divergent loops drifts.
- **Make everything MCP.** The Agent Bus already carries tiers, provenance
  and undo, which MCP has no concept of. MCP stays what it is: one way to
  *supply* tools (`mcp-bus`), not the internal contract.
- **A separate "agent daemon".** agentd already owns arbitration; the loop
  is a library so any surface can host it without another privileged
  service in the boot path.

## Consequences

- One place to fix routing, retries, budgets, and streaming — tonight's
  `lisa do`/Claude gap becomes structurally impossible.
- `forge`'s convergence guarantees (verifier, jail, turn budget) become
  available to every surface, including the Assistant.
- The pillars stop being an unused library; `harness-core` becomes the
  actual harness.
- Cost: phase 1 touches the code path that builds apps, so it lands behind
  the existing forge tests, unchanged, or it does not land.
- Sessions shared across surfaces means a conversation started in the
  Assistant can be resumed by `lisa agent --resume` — which is the feature
  users of Claude Code/Codex expect and currently the strongest argument
  for doing this at all.
