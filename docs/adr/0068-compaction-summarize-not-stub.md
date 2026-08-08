# ADR-0068 — context compaction: condense elided history instead of stubbing it

- **Status:** proposed
- **Date:** 2026-08-08 (revised 2026-08-08 after gauntlet critique)
- **Scope:** `libs/forge-harness` only (Lisa Coder). No shell/apps/OS.
- **Bar:** winners **condense** stale tool output rather than truncate it
  — OpenHands' "condenser" preserves the *thread* for a small-context
  model instead of blanking it
  ([OpenHands architecture](https://tensorfeed.ai/harnesses/openhands)).
- **Builds on:** ADR-0036 (the loop), ADR-0061 (Lisa Coder). Provenance
  lives *upstream* of this crate (ADR-0064/#305/#313, `bus-tools`) — see
  the decision.
- **Claims:**
  - `path:libs/forge-harness/src/agent.rs` — `elide_stale_tool_results` (stubs today), and `Message`

## Context

`elide_stale_tool_results` keeps the most recent few tool results under a
budget and **stubs the rest** — `"[elided N-char tool result — re-run
the tool if needed]"`. That protects the window but throws away the
signal a small local model needs, so it re-runs the tool it already ran.
The field condenses instead of blanking.

**The first draft of this ADR invented a mechanism it did not need, and
the gauntlet caught it.** It made the "load-bearing constraint" a
*provenance-carrying condensate*: each compacted entry would inherit the
union of the provenance tags of the results it condensed, to stop
compaction laundering `web` content into untagged history. That is
unbuildable here and defends a threat the architecture already
forecloses:

- `forge-harness` is **source-ignorant by design** (`agent.rs`: "the
  loop is ignorant of sources; provenance fencing and taint are the
  producer's obligation"). A `Message` tool result is a plain `String`
  with **no tag field**, and the crate does not depend on `bus-tools`.
- Taint is a **run-level, one-way set** that lives upstream
  (`bus_tools::Taint` — `Arc<Mutex<BTreeSet>>`, "a property of the
  conversation", added on entry, never removed). It is **not attached to
  the history string**.

So a condensate has no per-result tag to inherit, and — the point —
condensing a `content: String` **cannot lower or launder a tag it never
held**. Compaction is provenance-safe *because of* the architecture, not
because of a mechanism this ADR adds. The first draft's invariant was a
non-mechanism guarding a non-threat.

## Decision

**Condense by deterministic extraction; a model-summary is an opt-in
layer; and state plainly that provenance is not this crate's job.**

1. **Extractive compaction (default, deterministic).** When a result is
   elided, keep its structured skeleton instead of a stub: the tool and
   arguments, paths touched, exit codes, first/last N lines, error and
   warning lines, match counts. No model call, no new dependency — the
   signal a person scanning a transcript keeps, chosen by rule. This is
   the whole of the buildable, strictly-better change: it replaces a stub
   with a skeleton in one function.

2. **A model-summary layer, opt-in, for the thread extraction loses.**
   Extraction keeps the *inputs* (paths, errors) but not the *finding* —
   "explored auth, the bug is token expiry not signature" lives in
   reasoning, not tool output, and only a model summary keeps it. So a
   budget-driven model pass may condense further. This is the loop
   managing its **own** working memory — the model helping itself, not a
   guardrail between the model and the machine — so it is not a rule-6a
   surface. Off by default (a model call has a cost); extraction is the
   floor beneath it.

3. **Provenance stays upstream — which forecloses tag-laundering, and
   only that.** This crate does not tag condensates and must not start;
   its source-ignorance is deliberate. The run-level `Taint` reflects
   everything the run read *before* compaction and is never lowered by
   it, so a condensate cannot change the run's **tags** — the
   tag-laundering threat the first draft invented a mechanism against is
   closed by the architecture, needing no per-entry tag. That is the
   whole of what taint closes here, and the second gauntlet pass was
   right that the first revision overclaimed the rest: taint gates
   privileged **downstream calls**, not the model's own next within-tier
   action. So the **extraction floor** (item 1), being deterministic and
   rule-chosen, is fully safe; the **opt-in model-summary layer** (item
   2) carries a residual this ADR names rather than erases — see Limits.

## Consequences

- Small local models keep the thread across a long task without re-running
  tools — the buildable win, in one function, with no new crate coupling.
- The tag-laundering story is now *true*: compaction cannot launder a
  provenance tag because the tag was never on the thing it compacts, and
  the crate stays source-ignorant as designed. The corrected reasoning is
  recorded so no future revision re-adds a tag-union mechanism
  forge-harness cannot express — nor claims that closure covers more than
  tags.
- The model layer is where the semantic thread is kept; extraction is the
  zero-cost, fully-safe floor. The distinction is now explicit: the floor
  is deterministic; the summary layer trades a residual injection channel
  (below) for the thread, which is why it is opt-in and off by default.

## Limits

- **This is the smallest ADR of the harness set** — a heuristic upgrade
  to one function plus a correct statement of where provenance lives, not
  a grand architecture change. It is recorded because the first draft got
  the provenance reasoning inverted, and that inversion was worth fixing
  on the record.
- Extraction is heuristic per tool (knows `grep`/`read_file`/`run_tests`/
  `run_command` shapes; an unknown tool extracts to first/last lines).
  Best on the tools Lisa ships.
- The model summary can still drop a detail the next turn needed; it is
  opt-in and layered over the extract, never instead of it.
- **The model-summary layer opens a residual injection channel that taint
  does not close.** It has the model rewrite untrusted tool output into
  first-person planning prose and re-inject it, which strips the "this is
  data" frame and can steer the model's own next *within-tier* action —
  an inside-reachable manipulation (rule-6a's first test) that run-level
  taint does not gate, because taint gates privileged downstream *calls*,
  not the model's own reasoning. "Not a guardrail surface" means this
  lands here unpoliced, not that it is safe. This is precisely why the
  layer is **opt-in and off by default**: the deterministic extraction
  floor carries none of it, and turning the summary on accepts this
  residual knowingly. Closing it (e.g. keeping the summary in a
  data-framed envelope, or a deterministic-only condenser) is follow-up
  work, not a claim made here.
- Not implemented; records the corrected design so compaction condenses
  rather than stubs, provenance is left to the layer that owns it, and
  the summary layer's residual is named rather than waved through.
