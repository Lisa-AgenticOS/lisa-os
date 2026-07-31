# forge-harness — the agentic app-building loop

Spec: docs/PLAN.md §5.12.1. Milestone: M6. Governance: ADR-0004 (Flutter
lane), ADR-0027 (on-device SDK + launch), ADR-0029 (guardrails).

plan → edit files (jailed to project dir) → `dart analyze` / `flutter
analyze` → iterate. Pluggable backends: local coder models, a remote
provider, or any agent CLI over the same tool jail. Hot-reload preview and
VLM screenshot self-inspection are still ahead.

## What confines this loop

Nobody is watching it, so the boundaries are deterministic and live in
[`lisa-guard`](../lisa-guard/) rather than in prompt text (ADR-0029):

- **Files** — the agent reaches the directory it was spawned in and
  nothing above it. Absolute paths, `..`, and symlinks that leave the root
  at any depth are refused before any I/O.
- **Commands** — a small program allowlist, plus denied flags for the
  ones that can launch a child. `find` keeps its search predicates and
  loses `-exec`/`-delete`.
- **Verdicts** — there is no human in this loop, so a command that would
  need confirmation is refused, not assumed; the reason comes back as
  tool output for the model to route around.

**The limit, stated plainly:** none of that confines a *subprocess*.
`run_tests` invokes `cargo test` / `flutter test` over source the model
just wrote, which executes `build.rs` and test bodies as the user,
outside every guard above. So: **jailed for its own file tools,
(ADR-0029 phase 3); until it lands, run the forge loop on projects you'd
already be willing to `cargo test`.

Driven from the CLI (`cli/lisa`, `lisa forge`):

| verb | what it does |
|---|---|
| `lisa forge --setup` | install the pinned Flutter SDK to `/var/lib/lisa/flutter` — sha256-pinned tarball on x86_64, commit-pinned checkout on aarch64 (ADR-0027) |
| `lisa forge --flutter "…"` | scaffold a lisa_ui app (pubspec + `LisaApp` stub + smoke test) and run the loop with `flutter analyze` as the verifier |
| `lisa forge --build` / `--run` | `flutter build linux --release`, install the bundle under the forge apps dir, write the `.desktop` entry, optionally launch |

The workflow itself is a skill (`skills/build-lisa-ui-app/SKILL.md`,
ADR-0025), not hardcoded prose.

Status: **loop live** — plan→edit(jailed)→analyze→iterate converges
against real models and the scripted-backend test; the Flutter lane
scaffolds, verifies, builds and installs.

## Limits and open issues

- **Subprocesses are Landlock-confined on Linux** (ADR-0029 phase 3,
  #53). `cargo test` compiles and runs `build.rs` and test bodies the
  model just wrote; once `execve` has happened no Rust-level guard is in
  that process any more. The child is restricted to the project
  directory plus named build caches, with the toolchain read-only —
  applied in `pre_exec`, because a Landlock ruleset is inherited and
  cannot be relaxed, so applying it in the harness would confine the
  harness. Where Landlock is unavailable (macOS, older kernels) the
  subprocess runs **unconfined and says so** in its own tool output; a
  jail reported but not closed would be worse than none.

- **The Ledger is mandatory** (#129, closed). `AgentConfig` has no
  `Default`: constructing one requires deciding where the record goes,
  so an unledgered run does not compile. "No ledger entry, no action" is
  an invariant, not an option a caller can forget.
- **A jail escape is ledgered as `escaped`**, not `failed` (#126,
  closed) — a containment breach and a missing file must not look the
  same in the record.
- Tool arguments and outputs reach the Ledger through `preview_of`,
  which redacts credential-shaped text and strips control characters.
  That is a backstop, not a licence to preview secrets.
