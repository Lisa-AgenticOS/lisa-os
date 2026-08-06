# forge-harness — the agentic app-building loop

Spec: docs/PLAN.md §5.12.1. Milestone: M6. Governance: ADR-0047 (GJS +
GTK4 is the one toolkit), ADR-0050 (`lisa dev check` is the checker),
ADR-0029 (guardrails), ADR-0025 (one agent loop). ADR-0004/ADR-0027
describe the Flutter lane, which is parked.

plan → edit files (jailed to project dir) → run the verifier → feed its
findings back → iterate. Pluggable backends: local coder models, a
remote provider, or any agent CLI over the same tool jail. Hot-reload
preview and VLM screenshot self-inspection are still ahead.

**The verifier is the loop's own judgement, not the model's.** A
`DoneClaimed` is followed by a check rather than believed. Three arms
(`Verifier`): `Command { program, args }` — what `lisa forge` uses, with
`lisa dev check` as the program (ADR-0050 §4); `Dart` — the parked
lane's `dart analyze`; and `None`, for surfaces with no project at all
(the Assistant, `lisa assist`).

## The backend can be reached over a unix socket (#288)

`OpenAiBackend.url` takes either an `http(s)://` endpoint or
`unix:<path>`:

```rust
let mut backend = forge_harness::OpenAiBackend {
    url: "unix:/run/user/1000/lisa/inferenced.sock".into(),
    model: Some("qwen3-1.7b-instruct-q8".into()),
};
```

That exists because `lisa-harnessd` — which hosts the model — could not
be confined while it needed an IP socket. In a **user** unit
`IPAddressDeny=`/`IPAddressAllow=` are a no-op (an IP firewall is cgroup
BPF and needs root; the user manager logs "unit configures an IP
firewall, but not running as root"). The only directive that bites is
`RestrictAddressFamilies=`, a seccomp filter on `socket(2)` — so the
model host takes `AF_UNIX` and reaches `lisa-inferenced` on the socket
it already served for `lisa-contextd`.

`unix_http` implements this as a ureq **transport**, not as a second HTTP
client: chunked transfer-encoding, keep-alive and the SSE body reader
stay ureq's. That matters — the streaming lane really is
`Transfer-Encoding: chunked`, so the hand-rolled `Connection: close`
shape `lisa-contextd` uses for `/v1/embeddings` would have needed a
chunk decoder before it could stream a token. No new dependency was
added.

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
- **The shell** — `run_shell` (ADR-0036 §6, `shell_tool.rs`) takes an
  arbitrary line, so it gets the guard's *shell* reader rather than the
  argv one, a human's consent on **every** call, and — since #307 — the
  same Landlock ruleset `run_command` spawns under. Until then it had a
  working directory and nothing else: the broadest tool in the harness
  was the one with no kernel confinement, while the allowlisted tool
  beside it had had one since #53. The human is told which they are
  approving; `ShellRequest::confinement` carries it.

**The limit, stated plainly:** none of that confines a *subprocess*.
`run_tests` invokes `cargo test` (or the parked lane's `flutter test`)
over source the model just wrote, which executes `build.rs` and test
bodies as the user, outside every guard above. Landlock closes this on
Linux (ADR-0029 phase 3, below); elsewhere the subprocess runs
unconfined and says so, so run the forge loop on projects you would
already be willing to `cargo test`.

The same reasoning is why **`lisa dev check` does not run an app's own
suite**: the verifier is plain argv with none of that confinement, and a
checker that executes model-authored code in order to verify it would
hand the loop the escape the jail exists to prevent. `run_tests` reports
that a `tests/*.test.js` suite exists and that no JS runtime is on
`lisa-guard`'s command allowlist, rather than calling a real suite
"unrecognized".

Driven from the CLI (`cli/lisa`, `lisa forge`):

| verb | what it does |
|---|---|
| `lisa forge "a notes app"` | the default lane: write a GJS + GTK4/Adwaita app, verified each turn by `lisa dev check`. No scaffold and no toolchain — the source is interpreted, so an empty directory is a legitimate start and the checker says "no sources" until the model writes some |
| `lisa forge --flutter "…"` | **the parked lane** (ADR-0047): scaffold a `lisa_ui` Flutter app and verify with `flutter analyze`. Needs an SDK; nothing user-facing has ever been built this way |
| `lisa forge --setup` | fetch the pinned Flutter SDK for that lane into the user's own data dir — sha256-pinned tarball on x86_64, commit-pinned checkout on aarch64 (ADR-0027). Never needs `sudo` (#243) |
| `lisa forge --build` / `--run` | Flutter only: `flutter build linux --release`, install the bundle under the forge apps dir, write the `.desktop` entry, optionally launch |

The workflow itself is a skill (`skills/build-lisa-app/SKILL.md`,
ADR-0025), not hardcoded prose.

Status: **loop live** — plan→edit(jailed)→check→iterate converges
against real models and the scripted-backend test. `cli/lisa/tests/forge_verifier.rs`
runs the default lane's verifier as a real subprocess against a real app
tree, in both directions: an empty project reports findings, a
well-formed GJS app verifies clean.

## Skills scope what the loop may call (#57, #245)

`AgentConfig::skills` is the set of skills a run may LOAD — the same set
`read_skill` serves and the system prompt's catalog advertises
(`skills::load()` is the one search path, shared with `lisa skills`).

A skill's `tools:` frontmatter takes effect **when its body is served**,
not when its file exists on the machine:

- a skill sitting unread in `~/.local/share/lisa/skills` narrows
  nothing, so installing one cannot silently break an unrelated
  conversation;
- the restriction attaches at the moment untrusted text enters the
  context, and the loop applies the frontmatter of the body it *served*
  — a third-party skill (ADR-0049) cannot widen its own allowlist by
  saying so in its body.

Allowlists **intersect** across active skills, so activating a second,
unrestricted skill cannot restore the full tool set. A refused call comes
back as tool *output* naming what is allowed, not as an error that ends
the run.

This is scoping, not a guardrail in the ADR-0030 sense — what stands
between the model and the machine is the guard, the jail and the tiers.
The model can decline to load a skill, and gains nothing by declining
except the loss of the workflow.

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

  The writable set is `registry/` and `git/` under the package caches
  plus three named cargo lock files, **not** all of `~/.cargo` (#309):
  `~/.cargo/bin` is on the user's `$PATH` and `~/.cargo/config.toml`
  carries `runner = […]`, so a child that can write either has execution
  as the user, unconfined, the next time they open a terminal. The rest
  of `~/.cargo` and all of `~/.rustup` are **readable**, because a
  rustup toolchain lives there and `exec` happens after `restrict_self`.
  What that still leaves reachable is named in `confine.rs`:
  `~/.cargo/credentials.toml` is readable, and Landlock cannot subtract
  a path from a granted tree.

  `tests/confinement.rs` is the only thing that proves any of this: it
  runs a child that tries each escape and looks at what is on disk
  afterwards. It is Linux-only and that is not a gap it can close —
  Landlock is a Linux LSM, a macOS run executes nothing, and a
  confinement can only be witnessed where it exists.

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
- **The unix transport sits on `ureq::unversioned`** (#288). ureq says
  outright that its transport and resolver layers are not covered by its
  semver promise, so a ureq minor bump may break `unix_http`. It cannot
  break it silently — the crate stops compiling — and the alternative
  was either a new dependency (rule 7a's neighbourhood) or writing a
  chunked-encoding HTTP client by hand.
