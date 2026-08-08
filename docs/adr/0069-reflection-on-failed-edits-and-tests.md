# ADR-0069 — reflection on failed edits and failing verifier runs

- **Status:** proposed
- **Date:** 2026-08-08 (revised 2026-08-08 after gauntlet critique)
- **Scope:** `libs/forge-harness` only (Lisa Coder). No shell/apps/OS.
- **Bar:** winners turn a failure into a *structured* next step — a
  failed edit or a failing test feeds back "you tried X, it failed
  because Y" with escalating pressure to try differently, rather than a
  raw error the model re-hits. This is the reflection half of the agent
  loop the SWE-bench leaders rely on
  ([agentwiki SWE-bench](https://agentwiki.org/swe_bench)).
- **Builds on:** ADR-0061 steal 2 (`RefusalMemory` — the reflection
  ladder that already exists for guard soft-denials), ADR-0036 (the
  loop and the verifier gate).
- **Claims:**
  - `path:libs/forge-harness/src/tools.rs` — `RefusalMemory`, `edit_file`, `write_file`
  - `path:libs/forge-harness/src/agent.rs` — the verifier-findings feedback

## Context

forge-harness already has a reflection ladder — `RefusalMemory` (ADR-0061
steal 2) — but it fires only for **guard soft-denials**: a command the
guard asked about, repeated, costs more each time and mutes at three.
The two failures that dominate a coding loop are not covered:

1. **Failed edits.** `edit_file` returns actionable errors (0 matches,
   many matches, double-parent), but each is a one-shot string; a model
   that keeps trying a near-miss `old_string` gets the same error with no
   escalating signal that it is stuck, and no memory that this exact edit
   already failed.
2. **Failing verifier runs.** The verifier's findings are fed back each
   turn (ADR-0036), but flatly — a trajectory can loop on the same
   failing test, re-reading the same output, with nothing tracking that
   attempt N reproduced attempt N-1.

**The first draft of this ADR got reflection backwards, and the gauntlet
caught it.** It extended the ladder to edits and verifier findings but
kept the response *deterministic* — a fixed, count-indexed template, then
a mute at three — and called that a virtue (rule 6a). That is not
reflection; it is rate-limiting with escalating prose. The bar's power
(Reflexion, [arxiv 2303.11366](https://arxiv.org/abs/2303.11366)) is the
model generating a **new hypothesis** about *why* it failed and *what to
change*, before the next attempt. The first draft shipped the "stop when
stuck" half and omitted the "reason about why and try differently" half —
and worse, its own rule-6a justification proves the point against it: if
"the model helping itself" is not a guardrail (as ADR-0068's opt-in
summary establishes), then **model self-reflection on failure is already
rule-6a-clean** — the same category. The effective mechanism was excluded
for no valid reason. Two concrete failures the first draft caused:

- **Mute-abandons-where-the-bar-converges.** A tab/space `old_string`
  mismatch: 0 matches, the diff is invisible to the model, it retries a
  visually-identical string, mutes at three — the trajectory gives up two
  turns from the fix a reflection turn ("0 matches though the line looks
  present → likely whitespace; key off a distinctive token or use
  `replace_all`") would have found.
- **Numeric flapping dodges the mute forever.** A key over *normalised
  text* that strips numbers treats `expected 42, got 41` → `got 43` →
  `got 40` as three distinct failures, so the count never reaches the
  bound — the exact near-miss loop reflection exists to break walks
  straight past it.

## Decision

**The deterministic counter is a TRIGGER; the response is a model-authored
reflection turn — with the normalisation key inverted to catch flapping.**

1. **The counter triggers, it does not respond.** Failed edits
   (`edit_file`/`write_file`: 0-match, many-match, parent conflict) and
   verifier findings are counted per failure key, generalising
   `RefusalMemory`'s bookkeeping. That much stays deterministic — it is
   *detection*, not *judgement*.

2. **On the second identical failure, insert a reflection turn.** Before
   the next attempt, the model is prompted to first **state the most
   likely reason the last attempt failed and what it will change** — the
   Reflexion mechanism, and rule-6a-clean by ADR-0068's own logic (the
   model reasoning over its own failure signal, which is already in its
   context and already governed by the run's taint; it introduces no new
   untrusted content). This is the half the first draft omitted, and it
   is what matches the bar.

3. **The normalisation key folds the variable parts IN, not out.** A
   verifier finding's key canonicalises its *shape* including the
   asserted-vs-actual slot, so `expected X, got Y` for oscillating Y is
   **one** failure, not many — the inversion of the first draft's
   strip-the-numbers rule, which was the flapping hole. A genuinely
   different failure (different assertion, different file) is a different
   key, as it should be.

4. **Mute on the inverted failure key's count — not on comparing
   hypotheses.** A first draft of this item muted "when the model's
   proposed change repeats", and the gauntlet caught that comparing two
   model-authored free-text hypotheses is not the deterministic
   bookkeeping it was billed as: string-match is dodged by rephrasing,
   semantic-match needs the judge model the whole architecture avoids —
   and rationalisation *emits varied prose*, so that mute would be
   loosest exactly against the failure it must catch. So the mute is a
   bounded count on the **inverted failure key** from item 3 (default
   three). Because that key folds the variable parts *in*, the same
   underlying failure keeps hitting the same key however the model
   rephrases its reasoning — so flapping is counted and rationalisation
   (which keeps re-hitting one real failure while the prose wanders) is
   bounded, both by genuinely deterministic counting with no free-text
   comparison anywhere. Reflection (item 2) is the *response between*
   counts; the count, not a hypothesis-diff, is what ends the loop.

5. **One ladder, one policy, run-scoped.** `RefusalMemory` generalises to
   a `ReflectionMemory` over three families — denial, edit, verifier —
   with the trigger→reflect→mute-on-count shape in one place. A fresh
   trajectory starts clean, as `RefusalMemory` does.

6. **It composes with best-of-N.** Reflection raises a *single*
   trajectory's convergence; best-of-N (ADR-0065) runs several. The
   inserted reflection turn between counts is what keeps a trajectory
   from being abandoned two turns from a fix — the mute is still just the
   bounded inverted-key count of item 4, but a trajectory that reflects
   and changes approach produces a *different* failure (a new key), so it
   is the reflection, not a softer mute, that earns each of best-of-N's N
   a genuinely better attempt rather than a merely cheaper one.

## Consequences

- The coding loop gains *actual* reflection — a self-generated hypothesis
  on failure — which is the mechanism the bar's score comes from, not the
  rate-limiter the first draft mistook for it.
- The counter and the mute stay genuinely deterministic — both are counts
  on the inverted failure key, no natural-language comparison anywhere;
  only the *response between counts* is the model's, and that is
  rule-6a-clean by the same reasoning as ADR-0068's opt-in summary (the
  model helping itself over signal already in its context, adding no new
  untrusted content).
- Because the inverted key folds the flapping values in, the bounded
  count catches the two failures that dodge naive designs — numeric
  flapping and prose-varying rationalisation — with the same mechanism,
  and hands best-of-N and the eval harness a clean "this trajectory is
  stuck" signal that is a count, not a judgement.

## Limits

- Reflection can rationalise instead of fix — the known failure mode of
  naive self-reflection. The guard is the bounded inverted-key **count**:
  rationalisation keeps re-hitting one real failure (one key) while the
  prose wanders, so the count climbs and mutes regardless of how the
  reasoning is worded — no comparison of hypotheses is made or needed. It
  raises convergence odds, not the ceiling; it cannot manufacture a fix a
  weak verifier never asked for.
- **The count is blind to numeric convergence, which is the price of
  catching flapping.** Folding the asserted-vs-actual value into the key
  makes random flapping (`got 40, 43, 40`) one key — the fix — but it
  also makes a *converging* run (`got 39, 40, 41` toward `42`) one key,
  so a genuinely-progressing trajectory can be muted at the bound. The
  mitigations are honest, not complete: a converging run often crosses
  into a *different* failure (a new key) before the bound — helped by the
  reflection turn changing the next attempt — and best-of-N gives it
  other trajectories, but the count itself cannot see progress (letting a
  model "closer, one more" signal stay the mute would hand the model its
  own guard, which rule 6a forbids), and the
  bound (default three, inherited from `RefusalMemory`) is a tunable
  trade between abandoning a converging run early and looping a stuck one
  long, not a solved value. The first draft's opposite error (numbers
  stripped → flapping never bounded at all) was the worse one; this is
  the residual the fix accepts.
- The reflection turn costs a model call on the second failure; it is
  triggered by a real repeat, not every failure, so the common case (a
  failure fixed on the next try) pays nothing.
- Not implemented; records the corrected design so the ladder triggers
  real reflection, not a canned escalation, and the key catches flapping
  rather than being dodged by it.
