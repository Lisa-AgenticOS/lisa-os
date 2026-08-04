# forge-harness — the agentic app-building loop

Spec: docs/PLAN.md §5.12.1. Milestone: M6. Governance: ADR-0004 (Flutter
lane), ADR-0027 (on-device SDK + launch), ADR-0029 (guardrails).

plan → edit files (jailed to project dir) → `dart analyze` / `flutter
analyze` → iterate. Pluggable backends: local coder models, a remote
provider, or any agent CLI over the same tool jail. Hot-reload preview and
VLM screenshot self-inspection are still ahead.

## Attachments on the task turn (#209)

`AgentConfig::attachments` holds OpenAI content parts — an image a
person attached in a surface built on this loop. They ride on the
**task** turn and on nothing else, and `OpenAiBackend` writes that turn
as parts with the text FIRST:

```rust
let config = AgentConfig {
    attachments: vec![serde_json::json!({
        "type": "image_url",
        "image_url": {"url": "data:image/png;base64,…"},
    })],
    ..AgentConfig::new(ledger)
};
```

Per-run rather than per-message on purpose: the loop appends tool
results by the thousand, and there must be no path by which something
the model produced grows an image a human never showed it. The parts are
opaque `serde_json::Value`s and are forwarded verbatim — the same
decision `lisa-inferenced` made for `Content::Parts`. Empty (the normal
case) leaves the request byte-identical to a text-only one.

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

- **The streaming request shape is exported, on purpose** (#225,
  closed). `openai::streaming_request_body` is what
  `next_action_streaming` puts on the wire — tools attached,
  `stream: true` — and it is `pub` so `lisa-inferenced`'s own test suite
  can feed it to its own router. The two used to disagree (the harness
  streamed, the daemon's tools lane refused `stream: true` with a 400,
  and every Assistant run died as `backend: http status: 400`) with
  nothing on either side able to notice. Changing what this function
  emits now fails a test in the daemon.
- **A refusal carries the daemon's words** (#225). `backend_refusal`
  reads the response body instead of letting ureq collapse a non-2xx
  into `http status: 400` and throw away the sentence that said what was
  wrong. And an in-band `{"error": …}` SSE frame ends the turn as an
  error rather than as an empty `Done("")` — an engine that died halfway
  used to reach a person as a blank reply.
- **Stop stops the answer, not the action** (#227, closed).
  `AgentConfig.cancel` is a `Cancel` the loop consults before each turn,
  after the model answers but before its tool call is dispatched, and
  between frames while the answer is arriving. A tool that has STARTED
  runs to its end — killing a write halfway is how half-done actions
  happen. A stopped run returns `ForgeError::Cancelled`. The flag lives
  here rather than in a caller because harnessd's own copy of it
  (#227's actual defect) was set, read into a local, and never acted on:
  the loop it was meant to stop had no cancellation input at all.
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
