# ADR-0065 — best-of-N trajectories, selected by a verifier the trajectory cannot weaken

- **Status:** proposed
- **Date:** 2026-08-08 (revised twice 2026-08-08 after gauntlet critique)
- **Scope:** `libs/forge-harness` only (Lisa Coder). No shell/apps/OS.
- **Bar:** OpenHands' inference-time scaling — SWE-bench Verified 60.6%
  single-shot → 66.4% by generating N trajectories and selecting with a
  trained critic model
  ([openhands.dev](https://www.openhands.dev/blog/sota-on-swe-bench-verified-with-inference-time-scaling-and-critic-model)).
- **Builds on:** ADR-0061 (steal the good parts, keep the safety model),
  ADR-0036 (the verifier gate), ADR-0029/0030 (guardrails are
  deterministic code the model cannot reach), ADR-0064 (`TaintingWorkspace`),
  and #307/#309 (Landlock on tool subprocesses).
- **Sequencing:** this ADR is **gated on two others landing first** —
  ADR-0066 (localization) and the scored eval harness (ADR-0067). The
  gauntlet critique was right that shipping the flashiest published
  result before the measurement that proves it reproduces, and before
  the cheaper score-lever, is the wrong order. Best-of-N is written now
  so its *safety design* is settled; it is built last.
- **Claims:**
  - `path:libs/forge-harness/src/agent.rs` — the single-trajectory loop this wraps
  - `path:libs/forge-harness/src/lib.rs` — `Verifier`, the selection oracle
  - `path:libs/forge-harness/src/jail.rs` — the workspace bound a candidate is verified within

## Context

forge-harness runs one trajectory. One roll of the dice: a model that
localizes wrong, or writes an edit that passes but breaks a sibling, has
no second chance the harness can take for it. The field's answer is
best-of-N, and OpenHands' biggest published gain came from it.

**The first draft of this ADR got the selector wrong, and the gauntlet
caught it.** It proposed selecting the winner with the deterministic
`Verifier` and breaking ties by *smallest diff*, claiming that was more
rule-6a-aligned than OpenHands' learned critic. Two things were false:

1. **The verifier is reachable from inside the loop it judges.**
   `write_file`/`edit_file` accept any workspace-relative path, including
   the test files; `Verifier::Command` runs over the *edited* tree; and
   the jail does not confine the verifier subprocess. So a trajectory can
   delete the assertion that blocks it and be selected as a "passer".
   Best-of-N does not inherit this risk — it **multiplies** it: more
   attempts, more chances one games the check.

2. **"Smallest diff among passers" prefers the gamed patch.** A 3-line
   `if x == observed_case: return expected` passes a weak visible suite
   and is *smaller* than the 12-line correct fix, so the tiebreak ships
   the underfitting hack. rule 6a's real test is "is it reachable from
   inside?" — and the answer was yes, so the mechanism was not a
   guardrail at all.

A verifier is only a sound selector if a trajectory cannot weaken the
thing it is judged against. That is the whole of this revision.

## Decision

**Run up to N independent trajectories; select with a verifier each
trajectory could not touch; never prefer a smaller diff.**

1. **N independent trajectories, sequential for now.** Each is a full
   `forge_agent_with_tools`. True concurrency needs an isolated
   workspace copy per trajectory, which does not exist today
   (`Jail::new` canonicalises one pre-existing dir — there is no
   copy/COW in the tree). So this ADR ships **sequential** best-of-N
   over a snapshot/restore of the workspace between trajectories, and
   the concurrency claim is dropped until a copy mechanism is built
   (its own later ADR). Honest over aspirational: no shared-`current_dir`
   race is described as if isolated.

2. **The verifier's whole trusted computing base is pristine — defined
   by what the check actually depends on, not by a path convention.**
   The first revision said "restore the test tree", and the gauntlet
   showed that is too narrow: a trajectory can leave every `tests/*`
   file untouched and still weaken the check by editing the *test-runner
   invocation* (`package.json` `scripts.test`, `jest.config`,
   `pytest.ini`, `Cargo.toml`'s `[[test]]`/`dev-dependencies`), a
   fixture/golden/mock file, or an oracle module the assertions import.
   So "pristine" means the verifier's **trusted computing base** is
   restored to pre-trajectory state before selection: the acceptance
   tests, the runner config and the command that invokes it, and the
   declared fixture/golden/mock directories. Selection then runs the
   pristine command over the pristine TCB with only the trajectory's
   *non-TCB* edits applied on top. Landlock (#307/#309) is named for
   what it actually is — a **run-time** boundary keeping the verifier
   subprocess from reaching *outside* the workspace — not as tamper
   protection for in-workspace files, which is the restore's job.

   **Where this is a full guarantee, and where it is best-effort — stated,
   not blurred.** Against a **held-out** verifier the trajectory never
   had (the eval harness, ADR-0067: `fail_to_pass` tests supplied from
   outside the workspace), the TCB is unreachable by construction and the
   guarantee is complete. That is best-of-N's primary, sound use. In
   **production**, where the acceptance set is the repo's own tests, the
   TCB restore is a best-effort narrowing with one residual the gauntlet
   named and this ADR will not hide: an oracle module that lives under
   `src/` yet is what a test trusts cannot be told apart from a
   legitimate source fix, so a determined trajectory could still route
   through it. Production best-of-N therefore **flags any candidate that
   touched the declared TCB at all** and leans on the eval-case guarantee
   for its real assurance; it does not claim the full guarantee it cannot
   deliver against a self-hosted, editable suite.

3. **Selection, in order — never by size, never by test quantity.**
   - Discard trajectories that fail the pristine verifier.
   - **Prefer candidates that touched none of the TCB** (source-only
     fixes) over any that edited tests, runner config, or fixtures. A
     candidate that modified the thing that judges it is deprioritised
     and flagged, not rewarded.
   - **Smallest diff is not a criterion** (the first-revision gauntlet's
     underfitting bias — gone), and **"adds test coverage" is not a
     criterion either** (the second-revision gauntlet's finding: a hack
     plus one padding test that asserts the observed case would beat a
     correct fix with no new test — rewarding test *quantity* selects
     the padder). Additive tests remain welcome for *safety*, since they
     only raise the bar; they carry no selection *reward*.
   - A tie the harness cannot break on merit among source-only passers is
     **a decision the model must not make silently**: an attended run
     surfaces the tied candidates to the person; an unattended run fails
     safe — applies none, reports the tie — rather than shipping an
     arbitrary one. rule 6a, applied honestly.

4. **The all-fail case is defined.** When no trajectory passes the
   pristine verifier, none is applied and the run reports the failing
   trajectory with the **fewest pristine-verifier findings** as the best
   attempt, for reflection and human triage — "best" is now a named,
   deterministic quantity, not an undefined word.

5. **Every trajectory is Ledgered with its own id.** The `Event` schema
   gains a trajectory identifier so N interleaved runs are
   distinguishable and "why this patch" is answerable — the first draft
   claimed this while the schema carried no such field.

6. **N is bounded, budgeted, and opt-in.** Default small (3), capped by
   a token budget, and `N = 1` is today's behaviour bit-for-bit. Because
   the field shows best-of-N gains saturate (~5) and scale with base
   pass-rate and trajectory diversity — both lower for a local coder
   model — this ADR does **not** claim OpenHands' number is portable. It
   claims only the mechanism, and it is built after the eval harness that
   can measure whether it reproduces on Lisa's models at all.

## Consequences

- Best-of-N lands in a form where the selector is genuinely a guardrail:
  a candidate cannot weaken the bar it is judged against (restore +
  confinement), and the tiebreak no longer rewards the smallest gamer.
- Selection stays deterministic and outside the model — no critic model,
  no learned reward — which remains the ADR-0030 alignment. The
  difference from the first draft is that the claim is now *true*,
  because the verifier is pristine.
- The pristine-restore machinery is also exactly what a SWE-bench-mini
  runner needs (held-out FAIL_TO_PASS tests), so ADR-0067 and this share
  it rather than each inventing a snapshot.

## Limits

- **A weak acceptance suite still bounds the ceiling.** If the pristine
  tests only exercise the observed case, a special-case patch passes and
  best-of-N cannot know better — the single-shot failure mode unchanged.
  The revised tiebreak no longer *prefers* the special-case (it prefers
  source-only changes and rewards neither small diffs nor padded tests),
  but the harness cannot manufacture correctness a suite never asked for;
  it can only stop actively selecting for its absence.
- **Self-hosted verifiers are only best-effort ungameable, and only as
  complete as the TCB declaration.** The full "a trajectory cannot weaken
  the bar" guarantee holds against a held-out verifier (ADR-0067's eval
  case). In production the TCB restore closes the obvious routes (test
  files, runner config, fixtures) but not an oracle module disguised as
  source — and, more generally, *any* trusted file the TCB enumeration
  misses (an undeclared `conftest.py`, a CI/env helper, an undeclared
  golden) would read as source-only and be **preferred**, not flagged.
  A TCB declaration is only as complete as its enumeration; incompleteness
  silently rewards the toucher through the source-only preference. This
  does not apply to the held-out eval case at all, which is why production
  best-of-N leans on that case for its real assurance rather than on a
  claim the self-hosted selector cannot back.
- **Legitimate test edits are deferred, not lost.** Reverting acceptance-
  test edits for *selection* means a fix that genuinely requires updating
  a wrong assertion is judged against the old assertion and will not be
  auto-selected — it surfaces as a flagged trajectory for the person.
  That is the safe direction (a model rewriting the test that judges it
  is exactly what must not auto-ship) and the honest cost.
- **Sequential only** until a workspace-copy mechanism exists; N× wall
  time, not just N× tokens, for now.
- Not implemented; gated behind ADR-0066 and ADR-0067. This records the
  safety design so best-of-N is never built with an editable verifier.
