# lisa-guard

Deterministic guardrails for agent-issued actions. Spec: [ADR-0029](../../docs/adr/0029-hard-guardrails-for-agent-actions.md).

Probabilistic reasoning inside, logical guardrails outside. Nothing here
consults a model, reads a prompt, or depends on the model cooperating —
every answer is a pure function of the request, so the policy is
exhaustively testable and cannot be talked out of.

## The verdict

```rust
match lisa_guard::check_shell_line("sudo rm -rf /") {
    Verdict::Allow                   => run(),
    Verdict::Confirm { rule, reason } => ask_the_human(rule, reason),
    Verdict::Deny    { rule, reason } => refuse(rule, reason),   // final
}
```

`Deny` is **not overridable** — not by a confirmation dialog, not by
`--yes`, not by a persuasive prompt. That is the whole point: a tier
system whose every level is reachable by clicking "yes" is a speed bump,
not a guardrail. It is kept small (destroying the system or a whole home,
writing raw devices, escalating privilege, erasing the audit trail,
reaching a shell) precisely so it can stay absolute. Everything else that
merely deserves a second look is `Confirm`, because a `Deny` people
routinely need to work around is a `Deny` they will learn to disable.

## Three entry points

| | for | enforces |
|---|---|---|
| `contain(root, rel)` | every file tool | the agent reaches the directory it was given and nothing above it — absolute paths, `..`, and symlinks that leave the root at any depth |
| `check_command(program, args)` | the forge harness (argv, no shell) | program allowlist → per-program denied flags → shared rules |
| `check_shell_line(line)` | `lisa suggest`, before the string is printed into the user's shell buffer | splits on `;`/`&&`/`\|\|`/`\|`, unwraps `sudo`/`env`/`xargs`/`VAR=`, reads command substitution, then applies the same rules per segment |

## Callers

- `libs/forge-harness/src/jail.rs` — `contain` is the jail.
- `libs/forge-harness/src/tools.rs` — `check_command` gates `run_command`
  and `run_tests`. There is no human in that loop, so a `Confirm` verdict
  is refused rather than assumed; the reason goes back as tool output so
  the model can choose another route.
- `cli/lisa/src/terminal.rs` — `check_shell_line` screens every `lisa
  suggest` answer *before* it reaches stdout, which is what the Ctrl+G
  shell hook copies into the edit buffer.

## The corpus is the gate

`tests/corpus.rs` holds destructive attempts an agent might plausibly
emit and asserts none returns `Allow`, none of the catastrophic ones is
overridable, and everyday work still passes. It runs in `just test`.

**It is a floor, not a proof.** Green means those specific attempts are
stopped. It does not mean the agent cannot do damage — see below.

## What this crate does not do

It does not confine a subprocess. `run_tests` invokes `cargo test` /
`flutter test` over source the model just wrote, and that code executes
`build.rs` and test bodies as the user, outside every rule here. No
Rust-level policy can fix that; it needs an OS mechanism, and the plan is
Landlock (ADR-0029 phase 3). Until then the honest description of the
forge harness is: **jailed for its own file tools, unconfined for the
toolchains it invokes.**

## Adding a rule

Rules live in `src/rules.rs`, take one `Invocation`, and return a
`Verdict`; they are pure and order-independent, and the caller takes the
worst answer. Add the rule, add its unit test, and add at least one line
to the corpus — a rule with no corpus entry is a rule nobody will notice
regressing.
