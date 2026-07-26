# ADR-0029: Hard guardrails for agent actions — policy outside the model

- Status: accepted (phase 1 implemented 2026-07-26)
- Date: 2026-07-26
- Relates: ADR-0025 (one agent loop), PLAN §5.4 (Agent Bus), §5.10 +
  Appendix C (provenance, injection), §5.12.1 (forge tool jail), M5
  acceptance (injection suite), issue #23
- Supersedes in part: the "the same jail confines BYO agent backends"
  claim in `libs/forge-harness/src/jail.rs`, which was true of `..` and
  absolute paths and false of symlinks.

## Context

Lisa has two agent execution surfaces and only one of them has a safety
system.

**The Agent Bus (`daemons/agentd`) is genuinely guarded.** Tiers
(`read`/`write`/`destructive`), provenance escalation that fails closed on
an empty chain, ledger-before-dispatch, an undo journal, and a
merge-blocking injection gate all exist and are tested.

**The forge harness (`libs/forge-harness`) is where autonomous execution
actually happens, and it has none of that.** No tier, no confirmation, no
ledger, no undo. Its two guardrails were a path jail and a command
allowlist, and an audit of both found real escapes — not hypotheticals:

1. **The command allowlist pivots to a full shell.** `find` is
   allowlisted, and the argument check only rejected absolute paths and
   `..`. `run_command{program:"find", args:[".","-exec","sh","-c","<any
   string>",";"]}` passed every check: `sh`, `-c` and the payload are all
   relative-looking `Normal` path components. `find . -delete` wipes the
   project tree the same way. The doc comment claiming "no shell — there
   is no shell expansion to abuse" was wrong.

2. **The path jail is blind to symlinks.** `Jail::resolve` rejects `..`
   and absolute paths component-wise and then returns `root.join(rel)`
   without canonicalizing the result. `std::fs::write` follows symlinks,
   so a symlink inside the project pointing at `$HOME/.ssh` or `/etc`
   made `write_file` land outside the jail. The neighbouring
   `has_dart_sources` explicitly refuses to follow symlinks and cites
   issue #33 for it — the hazard was understood in one place and missed
   in the one that matters.

Chained, those are a two-step escape using only allowlisted tools:
create a symlink via `find -exec`, then write through it.

Three more findings are real but lower-order, and are phased below: the
forge loop writes no ledger entry for any mutation; `harness-core`'s
per-skill `tools:` allowlist is parsed, unit-tested, and **never called
from production code**; and `daemons/agentd/prompts/system-policy.md` is
versioned but loaded by nothing.

Finally, the one place a model-produced string reaches a human's Enter
key — `lisa suggest`, which assigns into the shell's `READLINE_LINE` /
`BUFFER` — sanitizes control characters and caps length, but never asks
whether the command is *destructive*. It will happily pre-type
`rm -rf ~/` and wait for the user to press Enter.

## Decision

**Probabilistic reasoning inside, logical guardrails outside.** No
guardrail may depend on the model's cooperation, on prompt text, or on a
heuristic that usually holds. Enforcement is deterministic, deny-by-
default, exhaustively testable, and lives in one place both surfaces
call.

That place is a new workspace crate, **`libs/lisa-guard`** — pure policy,
no I/O beyond path resolution, no async, no network. It answers exactly
one question, `Verdict`:

| Verdict | meaning |
|---|---|
| `Allow` | proceed |
| `Confirm{rule, reason}` | a human must say yes; `--yes` may satisfy it |
| `Deny{rule, reason}` | refused — **no confirmation and no flag overrides it** |

The irreversibility of `Deny` is the point. A tier system where every
level is reachable by confirmation is a UX speed bump, not a guardrail;
the user asked for the class of action that is simply not available, and
`Deny` is it.

### 1. Containment: the agent sees the directory it was given, and nothing else

`Guard::contain(root, rel)` replaces `Jail::resolve`. It keeps the
component check (no absolute, no `..`, no prefix) and adds the part that
was missing: it walks the path one component at a time and, whenever a
component exists on disk, canonicalizes it and re-asserts
`starts_with(root)`. A symlink at any depth that leaves the root is an
escape at the moment it is traversed, not after the write lands.

Writes additionally refuse a target whose own `symlink_metadata` says it
is a symlink, contained or not — the agent writes files, never through
links.

This is the rule the user named: **the agent has access to the directory
it was spawned in, and nothing above it.**

### 2. Command policy: allowlist the program, then police the argv

Program allowlisting stays, but it is no longer the whole story, because
an allowlisted program with an exec predicate *is* a shell. Three layers,
all deterministic:

- **Program allowlist** (`ALLOWED_COMMANDS`, moved into the guard so one
  crate owns policy).
- **Per-program argument policy.** Programs that can execute a child get
  an explicit denied-flag set — `find` loses `-exec`, `-execdir`, `-ok`,
  `-okdir`, `-delete`, `-fprintf`, `-fprint`, `-fls`. This closes the
  pivot at its source rather than trying to recognise the payload.
- **Destructive-pattern scan** over the rendered invocation, shared with
  the shell-line path below, as the backstop for anything the first two
  layers let through.

### 3. One corpus, one gate

`libs/lisa-guard/tests/corpus.rs` carries a corpus of destructive
attempts — root deletion, device writes, filesystem creation, fork bombs,
pipe-to-shell, recursive permission changes on system paths, privilege
escalation, history/journal wiping, and the two escapes above — and
asserts that **zero** of them return `Allow`. It is a merge gate in `just
test`, modelled on the existing `tests/injection-suite` gate.

A corpus is a floor, not a proof. It is stated as such here so nobody
reads a green gate as "the agent cannot do damage".

### 4. `lisa suggest` is guarded before it reaches the prompt buffer

The shell-line analyzer splits on `;`, `&&`, `||`, `|`, unwraps
`sudo`/`doas`/`env` prefixes, and runs the same pattern rules per
segment. A `Deny` suppresses the suggestion and says why; a `Confirm`
still pre-types it but prints the reason first, so the user's Enter is
informed rather than reflexive.

## What was rejected

- **Prompt-level rules only** ("never run destructive commands" in the
  system prompt). This is what the repo already had in
  `agent.rs`/`terminal.rs`, and it is exactly the guardrail the slide
  above argues against: it lives inside the probabilistic system and
  fails whenever the model is confused or steered.
- **Escalating everything to a confirmation dialog.** Confirmation
  fatigue converts a guardrail into a click-through, and it puts the
  catastrophic and the routine in the same widget. Hence a `Deny` class
  with no override.
- **A blanket "no interpreter name anywhere in argv" rule.** It would
  reject `grep -n bash file` and teach users to route around the guard.
  The per-program denied-flag set is narrower and does the real work.
- **Reusing the agentd tier machinery for the forge loop.** Tiers there
  are *declared by app manifests* and unvalidated; a manifest can label a
  delete as `read` and get silent execution. That path needs its own fix
  (phase 2) and is not a foundation to build on today.

## Phases

**Phase 1 (this ADR, implemented):** the guard crate, the containment
fix, the argv policy, the `lisa suggest` gate, the corpus, CI.

**Phase 2 (filed as issues):**
- Ledger every forge mutation (`forge.write`, `forge.command`) so the
  autonomous loop is auditable — today it writes nothing.
- Enforce `harness-core`'s per-skill `tools:` allowlist at dispatch; it
  is parsed and ignored.
- Validate manifest-declared tiers at load, and verify D-Bus provenance
  against peer credentials instead of trusting the caller's assertion —
  any session-bus client can currently send `provenance:["user"]` and get
  the trusted path.
- Load `daemons/agentd/prompts/system-policy.md`, or delete it.

**Phase 3 (the honest limit):** *none of the above confines a
subprocess.* `run_tests` runs `cargo test` / `flutter test` over source
the model just wrote, which executes `build.rs` and test code as the
user, outside every guard in this ADR. A Rust-level policy cannot fix
that; it needs an OS mechanism. **Landlock** is the right fit — an
unprivileged kernel LSM that restricts a process's filesystem view to a
named set of paths, which is precisely "the directory it was spawned in."
Until that lands, the forge harness must be described as *jailed for its
own file tools and unconfined for the toolchains it invokes*, and this
ADR says so rather than letting the README imply otherwise.

## Consequences

- Two working escapes are closed, with regression tests naming them.
- Policy lives in one auditable crate instead of being spread across a
  doc comment, a heuristic loop, and a prompt paragraph.
- `find` loses its action predicates. Search still works; deletion and
  execution via `find` do not. This is a deliberate capability loss.
- The `Deny` class means some legitimate operation will eventually be
  refused with no way around it. That is the trade being made, and the
  reason `Deny` is kept small and `Confirm` carries the rest.
- The forge harness remains unconfined at the subprocess boundary until
  phase 3. That is now written down instead of assumed away.
