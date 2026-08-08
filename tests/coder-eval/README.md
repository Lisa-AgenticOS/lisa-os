# coder-eval — the committed tier of the coder eval harness (ADR-0067)

## What it does

Scores the forge-harness agent loop against a committed set of tiny Rust
fixtures, SWE-bench-shaped: each fixture is a real planted bug with
`fail_to_pass` tests (red before, must be green after) and
`pass_to_pass` tests (green before, must stay green). The score is
`resolved / total`; a broken environment or a crashed run is a counted
FAIL, never a hidden skip.

Two lanes:

- **oracle lane** (`tests/oracle.rs`) — runs in CI with `just test`,
  model-free. It applies each fixture's known-good `patch.json` through
  the real `WorkspaceTools` tool path and grades. It proves the
  machinery with three controls: positive (the patch resolves), negative
  (an untouched tree grades unresolved), and restore (rewriting the
  tests does not beat the grader).
- **model lane** (`src/main.rs`) — opt-in, needs a live model:

  ```
  cargo run -p coder-eval -- --url http://127.0.0.1:7778 \
      --model remote:anthropic:claude-haiku-4-5-20251001 \
      --out report.json
  ```

  Runs every fixture through `forge_agent_with_tools`, restores the
  pristine tests, grades, and prints `coder-eval [model]: N/M resolved`.

## How it works

A fixture directory is `task.json` (prompt + test names), `patch.json`
(the known-good fix, full files), and `tree/` (a self-contained,
zero-dependency crate with `[workspace]` detached). The runner stages
`tree/` into a temp directory, checks the **sanity gate** (the bug must
reproduce: fail_to_pass red, pass_to_pass green), runs the lane, copies
the pristine `tests/` back (**restore**, ADR-0065's mechanism), and
grades by running each named test with `cargo test --offline -- --exact`.

## How to extend it

Add a directory under `fixtures/` with the three parts above. The
oracle test discovers it automatically and will fail the suite if the
bug does not reproduce or the patch does not resolve it — a fixture
nobody can solve, or one that is already solved, never enters the
denominator silently.

## Limits

- **This is the regression/smoke tier only.** At this N the binomial
  noise floor is ~±9pp, so the model lane can catch "the loop broke" but
  must not certify a +6pp improvement (ADR-0067 §1). The public
  SWE-bench adapter tier — the improvement measure — is **not built**;
  ADR-0065/0066's improvement claims wait for it.
- Restore reverts test *files*, not knowledge: a trajectory that read
  the tests and special-cased the inputs still grades resolved
  (ADR-0065's named residual; the public tier's held-out grading is the
  answer).
- The model lane's number depends on the model behind the URL; compare
  runs only against the same model, and prefer deltas over absolutes.
- Fixture crates must stay zero-dependency so `cargo test --offline`
  works everywhere CI runs.
