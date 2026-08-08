# ADR-0066 — localization: regex, a deterministically-ranked repo map, and steerable lookup tools

- **Status:** proposed
- **Date:** 2026-08-08 (revised 2026-08-08 after gauntlet critique)
- **Scope:** `libs/forge-harness` only (Lisa Coder). No shell/apps/OS.
- **Bar:** localization quality is the **dominant SWE-bench score
  correlate**, and the reference floor — Aider's repo map — hits its
  accuracy with **deterministic graph math**: files/symbols are nodes,
  references/definitions edges, and a **personalized PageRank** biased
  toward task-mentioned identifiers ranks which symbols matter, filling a
  token-budgeted map top-first
  ([aider repo-map](https://aider.chat/docs/repomap.html)). SWE-Debate /
  SWE-Explore add debate on top; the floor is arithmetic, no LLM scoring.
- **Builds on:** ADR-0061 (`AmbientSource` seam), ADR-0036 (jailed
  tools), ADR-0064 (`TaintingWorkspace`), ADR-0029/0030 (guardrails are
  deterministic code outside the model).
- **Claims:**
  - `path:libs/forge-harness/src/tools.rs` — `grep` (literal today), the tool surface
  - `path:libs/forge-harness/src/agent.rs` — the `AmbientSource` turn-boundary seam

## Context

`tools.rs::grep` matches a literal substring, capped at `MAX_GREP_HITS`
in walk order, with no line-length cap. A model cannot ask for a regex,
"the definition of X", or "the callers of Y"; it greps a guess and reads
whole files. Localization — finding the right place cheaply before
editing — is what the research says separates winners, and it is
forge-harness's weakest area.

**The first draft of this ADR got the central design wrong, and the
gauntlet caught it.** It shipped the two cheap halves — regex, and a
*flat* "what exists" symbol map — and refused the one half the bar is
actually made of, **ranking**, on the grounds that ranked localization
is "a second model… rule 6a's line." That conflated two different
things. Aider's ranking is *deterministic graph math*: PageRank over the
reference graph is arithmetic the model cannot reach — which is exactly
what rule 6a **protects** (deterministic code outside the model), not
what it forbids. Rule 6a bans a *judge model* in a guardrail; it says
nothing against deterministic relevance ordering. The first draft left
the single best available deterministic win on the table by misreading
its own constitution, and handed the least-capable model (`qwen3-0.6b`)
an unranked dump to sort — backwards.

## Decision

**Deterministic, bounded, steerable localization: regex search, a
graph-ranked repo map, and lookup tools the model can drive — no judge
model anywhere.**

1. **`grep` gains regex**, on Rust's `regex` crate (linear-time by
   construction — finite automata, no backtracking — so a
   model-or-attacker pattern cannot ReDoS). Literal search stays the
   default spelling; regex is opt-in per call. Bounded on all three
   axes the first draft only asserted: `RegexBuilder::size_limit` +
   `dfa_size_limit` against compile-time blowup, a **line-length cap**
   (which today's `grep` lacks), and the existing match cap. A strict
   superset — nothing that worked stops. (`regex` is not yet a workspace
   dependency; adding it is part of this.)

2. **A repo map, ranked by deterministic graph math.** Build a
   symbol/reference graph over the jailed workspace (definitions and
   references, via tree-sitter where a grammar exists, a regex-tag
   fallback elsewhere), then rank nodes by a **personalized PageRank**
   whose personalization vector is biased toward identifiers mentioned
   in the task text — Aider's mechanism exactly, and every input is
   deterministic: graph centrality, symbol-to-task-text proximity, and
   git recency. No model scores anything; the ranking is reproducible
   and auditable. The map is filled top-first under a token budget, so a
   large repo yields the *relevant* head, not a bigger haystack.

3. **Localization is steerable tools, not only an ambient push.** The
   `AmbientSource` seam (`fn take(&self) -> Option<String>`) is a
   one-way broadcast the model cannot query, and re-injecting a full map
   at every turn boundary spends the very context it meant to save.
   So the map's ranked head is offered once as an ambient starting hint,
   and the model *steers* localization through tools it calls on demand:
   `outline(path)` (a file's symbols), `find_refs(symbol)` (callers/uses
   from the graph), and `expand(path)` (widen the map around a file).
   Interactive, budgeted, and — being reads over the jail — tier-classed
   and `file`-tainting under ADR-0064 like any other read. Winners let
   the agent query localization; this does too.

4. **Everything stays inside the jail, deterministic, and Ledgered.**
   The graph is built over only what `Jail` bounds; a lookup is a read;
   no step introduces a model that judges relevance. The score lever the
   field says matters most is pulled with math, not a critic.

## Consequences

- The mechanism behind the reference's accuracy — deterministic,
  personalized graph ranking — is adopted rather than admired-then-
  declined. rule 6a is satisfied *correctly*: the ranker is arithmetic
  the model cannot reach, which is the rule's intent, and the corrected
  reading is written down so the next ADR does not re-ban deterministic
  math as if it were a judge.
- Small local models gain the most: a ranked head plus steerable lookup
  is how a 0.6b model punches above its context window, and it no longer
  has to sort an unranked dump.
- Composes with best-of-N (ADR-0065): better localization raises each
  trajectory's hit rate, and — per that ADR's revised sequencing — this
  lands **before** best-of-N, because it is the cheaper, larger score
  lever.

## Limits

- The map is a definition/reference graph, not full data-flow: it ranks
  "where X is and what touches it" well, and does not model value flow.
  Aider's floor is the same shape; deeper analysis is a later ADR if the
  eval shows it pays.
- tree-sitter grammars are per-language; languages without one get the
  coarser regex-tag fallback. Best on the languages Lisa is written in,
  graceful elsewhere — stated, not hidden.
- PageRank over a huge repo has a build cost; it is cached and rebuilt on
  workspace change, and budget-capped so the graph itself cannot grow
  unbounded. The cost is paid once per change, not per turn.
- **Cache invalidation is a design detail deferred to implementation,
  not hand-waved:** what *detects* a workspace change is ADR-0064's
  `TaintingWorkspace`, which already sees every write the tools make —
  it is the seam that marks the graph stale. The failure direction is
  the safe one (a stale map is a weaker hint, never a wrong grader), so
  this is an implementation choice, not a correctness risk to settle
  here.
- Not implemented; gated before best-of-N. This records the corrected
  design — deterministic ranking is in, an ambient-only dump is out — so
  localization is built as a ranked, steerable capability the first time.
