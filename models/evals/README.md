# models/evals — golden-output evals

Spec: docs/PLAN.md §11. Milestone: M1+.

Fixture prompts + semantic-similarity scoring for the task APIs, tracked across model updates; eval-score diffs ship with every catalog refresh (§6).

Status: **not started** — scaffold placeholder. Read the spec section (and CLAUDE.md rules) before writing code here.


## addressed_intent_eval.py (ADR-0011 Phase-2 gate)

Labeled utterances → the addressed-intent classifier → accuracy,
false-accept (fires when NOT addressed — the privacy failure), and
recall (responds when addressed — the utility side). The wake-word-free
Ambient mode ships only when a model clears **false-accept < 5% AND
recall > 70%**. Baseline 2026-07-23 on Gemma-1B: the small model can't
calibrate the boundary (over-conservative → 0% false-accept but rejects
real requests), which is exactly why "Hey Lisa" is the default. Run:
`python models/evals/addressed_intent_eval.py` against a running daemon.
