# ADR-0067 — a scored eval harness: a regression gate and a sized improvement measure

- **Status:** proposed
- **Date:** 2026-08-08 (revised 2026-08-08 after gauntlet critique)
- **Scope:** `libs/forge-harness` (+ a `tests/`-side runner). No shell/apps/OS.
- **Bar:** the SWE-bench evaluation harness — per-task **containerised
  environments** (setup rot is why Docker exists), `FAIL_TO_PASS` +
  `PASS_TO_PASS` grading, and resolve-rate over enough tasks (Lite 300,
  Verified 500, full 2,294) that a single-digit delta is **statistically
  legible** ([swebench.com](https://www.swebench.com/viewer.html),
  [github.com/SWE-bench/SWE-bench](https://github.com/SWE-bench/SWE-bench)).
- **Enables:** the *improvement* claims of ADR-0065 (best-of-N) and
  ADR-0066 (localization). This ADR is the prerequisite both name.
- **Builds on:** ADR-0065's TCB snapshot/restore (shared), ADR-0036
  (verifier gate), #307/#309 (Landlock), ADR-0061.
- **Claims:**
  - `path:libs/forge-harness/src/agent.rs` — the loop under measurement
  - `path:cli/lisa/tests/forge_verifier.rs` — the single test this supersedes as the measure

## Context

You cannot win a gauntlet you cannot score. The first draft of this ADR
copied SWE-bench's *shape* (fail_to_pass/pass_to_pass, exit-code scoring,
a baseline diff) but the gauntlet caught it failing the two things that
make the shape a *measuring stick*: enough tasks to resolve the effect it
must gate, and reproducible per-task environments so the score means the
same thing twice. It also used a skip-excluded denominator that would
turn missing reproducibility into invisible score inflation, and it
called restore-based grading "held-out", which it is not.

The decisive numbers: ADR-0065's headline effect is +5.8pp
(60.6→66.4), and localization gains are similar-order. A tens-of-tasks
set has a binomial noise floor of ~±9pp (30 tasks at p≈0.6), so a +6pp
move is under one standard deviation — the instrument cannot resolve the
effect it was specified to gate. Building a centimetre ruler to certify
millimetre changes and asserting it works is the defect.

## Decision

**Two tiers with distinct, honestly-sized roles — a regression gate that
runs everywhere, and an improvement measure sized to the effect.**

1. **Committed tier — a regression + smoke gate, not an improvement
   gate.** A small set of **Rust** fixtures (the workspace's own
   language, so `fail_to_pass` actually *executes* in the CI/unit lane —
   the first draft's GJS fixtures could not, needing gjs/GTK4/a display).
   Each is a real bug with a failing `cargo test` that must go red→green
   and `pass_to_pass` that must stay green. It runs offline, in CI, every
   commit. Its job is to catch a **large regression** (a change that
   drops resolve-rate by more than the ±9pp noise floor, or breaks the
   loop entirely) — a job a small N does well. It is **explicitly not**
   allowed to certify a +6pp improvement, and it does not gate 0065/0066
   on one.

2. **Public-adapter tier — the improvement measure, sized to the
   effect.** An opt-in adapter for **SWE-bench Lite/Verified**, pinned by
   hash. This is where 0065/0066 prove they move the number, at N large
   enough to resolve it: to detect +6pp at p<0.05 needs on the order of
   **hundreds** of tasks — an order-of-magnitude sizing anchored to the
   real benchmark sizes (Lite 300, Verified 500), named as that rather
   than dressed up as a full α/power MDE — which is exactly why the
   public set, not the committed dozen, is the improvement gate. The ADR's rule is now precise: *an
   improvement ADR ships when it moves the public-tier score by more than
   that tier's computed confidence interval*, and the committed tier is
   the fast regression guard between those runs.

3. **Real per-task environments, or a counted failure — never a hidden
   skip.** The committed tier's Rust fixtures build in the container CI
   already uses (the same substrate as the image build), so their
   environment is reproducible by construction. The public tier
   provisions each task's dependencies in that container; provisioning
   needs network *at setup*, so the public tier runs only in the
   network-permitted eval lane, never inside the no-egress runtime jail
   (rule 5 is not bent — the eval lane is a different, deliberately
   networked context, and says so). **A task whose environment cannot be
   provisioned is a FAIL, counted in the denominator**, not a skip. The
   score is `passed / total`. The only exclusions are genuinely
   not-applicable tasks (e.g. wrong arch), reported loudly and separately
   — so environment rot shows up as a falling number, never a shrinking
   denominator.

4. **"Held-out" means held-out; "restore" is named as best-effort.** The
   public tier is *true* held-out: SWE-bench's `FAIL_TO_PASS` are applied
   **after** the agent finishes, from outside the workspace the model saw
   — so overfitting to the grader is impossible, and this is where the
   trustworthy absolute number comes from. The committed tier uses
   ADR-0065's **restore** (tests reverted to pristine before grading),
   which stops a trajectory *deleting* the assertion but not *reading*
   it and special-casing the input — the same overfit residual ADR-0065
   names. The two are no longer conflated: restore ≠ held-out, and the
   ADR says which tier has which property.

5. **A score report and a baseline diff.** Machine-readable per-task
   pass/fail (tokens, wall-time, trajectory count), a summary line, and a
   committed baseline per tier so a run reports the delta with the tier's
   confidence interval attached — "localization: +6 tasks on the public
   tier, CI ±4" is a claim; "+1 on the committed dozen" is explicitly not.

## Consequences

- The gauntlet gets a number that can bear the weight put on it: large
  regressions caught cheaply every commit, real improvements measured
  where the statistics support the claim. An ADR that moves neither does
  not ship, and the ADR is honest about which tier certifies which.
- The container substrate and the restore/held-out machinery are shared
  with the image build and ADR-0065 respectively — built once.
- rule 5 is preserved by construction: the networked public tier is a
  separate eval context, never the runtime jail.

## Limits

- The public tier needs network + opt-in + real compute; it is not a
  per-commit gate, it is a milestone measurement. The committed tier is
  the per-commit signal, and it can only see large moves. This split is
  the honest shape of "a small offline gate plus a real benchmark",
  not a single ruler pretending to both jobs.
- Absolute parity with a full-benchmark score is a public-tier,
  large-N claim only; nothing here lets the committed dozen stand in for
  it.
- Scoring is only as honest as the held-out tests on the public tier and
  as the restore's TCB coverage on the committed tier (ADR-0065's
  residual). Same limit, same reason — shared verifier.
- Not implemented; built first of the three so the other two have a
  sized measure to prove themselves against.
