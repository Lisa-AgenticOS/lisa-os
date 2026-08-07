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

## Four entry points

| | for | enforces |
|---|---|---|
| `contain(root, rel)` | every file tool | the agent reaches the directory it was given and nothing above it — absolute paths, `..`, and symlinks that leave the root at any depth |
| `check_command(program, args)` | the forge harness (argv, no shell) | program allowlist → per-program denied flags → shared rules |
| `check_shell_line(line)` | `lisa suggest`, before the string is printed into the user's shell buffer | splits on `;`/`&&`/`\|\|`/`\|`, unwraps `sudo`/`env`/`xargs`/`VAR=`, reads command substitution, then applies the same rules per segment |
| `judge_action(action, grant)` | the **Agent Bus**, before a tool call is dispatched or parked | the same catalogue, applied to `(tool, arguments, grant)` |

## The Agent Bus verdict (#251, #252)

The bus used to top out at *destructive → confirm-modal*, so a model that
**typed** `rm -rf /` was refused with no dialog and a model that **called
a tool** doing the same thing got a modal with an Allow button. Same
damage, two policies. `judge_action` is the second policy deleted.

```rust
let verdict = lisa_guard::judge_action(
    &lisa_guard::Action {
        app_id: "app.lisaos.Probe244",
        tool: "tidy_up",                       // the name proves nothing
        class: lisa_guard::Class::Delete,      // the manifest CEILING
        args: &serde_json::json!({"path": "/"}),
    },
    &grant,                                    // home, uid, workspace, trigger
);
assert!(verdict.is_hard_no());
```

Four verdicts:

| | meaning | surface |
|---|---|---|
| `HardNo` | no legitimate agent workflow requires this, ever | a dialog that **reports** — one button, no approving control |
| `No` | out of bounds *for the current grant* | refused, naming the scope that would permit it; nothing in the dialog widens it |
| `Ask` | in bounds and consequential | ask, with the effect in plain language |
| `Ask { may_remember: true }` | …and "always allow" may be offered | never on an untrusted chain |

**HARD NO is a property of the action; NO is a property of the current
permission.** Collapsing them makes refusals overridable or ordinary
out-of-scope work permanently impossible.

The load-bearing part is that the verdict is **computed from the target,
not declared by the manifest**. A tool's tier is the ceiling the app
asked for; where a given call lands beneath it is decided by where it
points. One `delete_file` yields `Ask` in the working folder,
`scope.hidden_folder` at `~/.ssh/id_rsa`, `scope.outside_home` at `/tmp`,
`fs.not_yours` at `/home/alice`, and `rm.system_path` at `/`.

Every string in the arguments is read, at any depth, whatever its key is
named — an argument's NAME is the app's choice, and the app may be what
we are defending against. Every path is judged in **both** spellings,
lexical and symlink-resolved: canonicalising alone moves `/etc` to
`/private/etc` on macOS, and resolving nothing at all misses a workspace
symlink pointing into another user's home.

### The scope ladder

| where | read | write | delete |
|---|---|---|---|
| agent scratch (agent-owned) | yes | yes | yes, silent |
| working folder | yes | yes | confirm |
| home content dirs, `trigger: prompt` only | yes | yes | confirm |
| hidden folders (`~/.*`) | no | no | no |
| outside `~` | NO | NO | NO |
| the seven HARD NO categories | never | never | never |

### Rule ids

`exec.shell`, `escalate.privilege`, `fill.password_field`,
`disk.raw_write`, `rm.system_path`, `audit.erase`, `fs.not_yours` are
HARD NO. `scope.hidden_folder`, `scope.outside_home`,
`scope.unattended_reach` are NO. Four of them are the shell guard's own
ids, deliberately: one vocabulary, so the Ledger shows one rule rather
than two spellings of it.

`lisa guard list` prints both tables. Bus rules are **not** relaxable —
`judge_action` does not read `Overrides` at all, and `lisa guard allow`
refuses a bus-only id rather than printing "relaxed" for something that
is still enforced.

## The approval verdict (#216)

`judge_approval` answers a different question from `judge_action`: not
*what is this call*, but **who may release it** once the bus has parked
it. It became load-bearing the moment an agent loop was offered a
write-tier tool, because the peer most eager to release such a call is
the process hosting the model that asked for it.

```rust
match lisa_guard::judge_approval(&Approval {
    approve: true,
    is_requester: false,      // a DIFFERENT connection…
    answerer_is_requesters_process: true, // …of the SAME process (#289)
    owns_consent_name: true,  // the broker's answer, not a claim
    answerer_is_consent_program: false, // /proc/<pid>/exe says otherwise
    requester_hosts_a_model: true, // /proc/<pid>/exe, not a message
    class: ConfirmClass::Modal,
    brokered: true,
}) {
    ApprovalVerdict::Refused { rule, .. } => assert_eq!(rule, "consent.same_process"),
    v => panic!("{v:?}"),
}
```

Three rule ids, all in `BUS_RULES`:

| rule | when | relaxable |
|---|---|---|
| `consent.self_approval` | a model host approving a call it made — any tier, broker or not | never (`HARD_NO_RULES`) |
| `consent.no_surface` | a modal with no independent dialog to answer it (#244) | never, but starting the dialog resolves it |
| `consent.same_process` | the process that parked a call approving it over a second connection (#289) | never (`HARD_NO_RULES`) |

**A name is not an identity, and neither is a connection.** Both of the
older fields say less than they look like they say, and #289 is what came
of reading them as more:

- `is_requester` compares unique **bus names**, so a `false` means "a
  different socket", not "a different process". One process may hold as
  many as it likes.
- `owns_consent_name` is the broker's unforgeable answer to
  `GetNameOwner` — and `session.conf` ships `<allow own="*"/>`, so it
  says only that this peer called `RequestName` first.

`answerer_is_requesters_process` (pidfd-pinned pids, `lisa_peer::Process`)
and `answerer_is_consent_program` (`/proc/<pid>/exe` against an
allowlist) are the two facts that make the sentence at the top of
`src/consent.rs` — *a process that hosts a model may never approve a call
it made* — say **process**.

Every field of `Approval` is transport-derived. Not one is read from a
message, which is what makes this outside the boundary rather than
inside it (ADR-0030 §2). What the requester keeps is the right to
**withdraw** its own call, because withdrawal causes no action — a
guardrail that stopped it would be aimed at the wrong side of the line.

The corpus tables are `CONSENT_MUST_REFUSE`, `CONSENT_MUST_ALLOW`, and
`CONSENT_MUST_LEARN_NOTHING` — the attempts that must be refused
*without* a rule id, because naming one would confirm the call exists
— in `tests/corpus.rs`. The second exists because a corpus of refusals alone
cannot tell a working boundary from a broken one: without it a
`judge_approval` that refused *everything* would pass.

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

**Agent scratch does not exist.** `Grant::scratch` is the only row of the
ladder where silent deletion is defensible, and nothing in Lisa builds
one yet — `harnessd` has exactly one notion of a working folder and it is
the owner's data. It is `None` in production. The field is here so nobody
grants that property to a workspace by accident.

**"Always allow" is a decision, not a memory.** `may_remember` says
whether a surface *may* offer it; there is no store behind it. Persisting
it means reusing the portal's append-only grant log (#252) and the
Settings page that revokes it (#253), and neither is built.

**The password-field rule sees the selector, not the field.** #212 landed
`fill(selector:"#q")` in a field named `password`, because the page owned
the JS world; no string rule here would have caught that. Refusing a
field the browser has *resolved* as `type=password` is the other half,
and it belongs in Surfer (#260).

**`fs.not_yours` is defence in depth, not the mechanism.** Unix
permissions do most of this work and get the credit: agentd runs as one
user, so an agent acting as `lisa` cannot unlink `/home/alice/notes.txt`
— it lacks permission and no policy is consulted. The rule matters where
the kernel does not object: elevated contexts, shared group directories,
network mounts, world-writable paths.

It does not confine a subprocess. `run_tests` invokes `cargo test`
over source the model just wrote, and that code executes
`build.rs` and test bodies as the user, outside every rule here. No
Rust-level policy can fix that; it needs an OS mechanism, and the plan is
Landlock (ADR-0029 phase 3). Until then the honest description of the
forge harness is: **jailed for its own file tools, unconfined for the
toolchains it invokes.**

## Adding a rule

Shell rules live in `src/rules.rs`, take one `Invocation`, and return a
`Verdict`; they are pure and order-independent, and the caller takes the
worst answer. Bus rules live in `src/action.rs`, take an `Action` and a
`Grant`, and return an `ActionVerdict`; a call is judged by its worst
argument. Add the rule, add its id to `BUS_RULES` (and `HARD_NO_RULES` if
it is one), add its unit test, and add at least one line to the corpus —
a rule with no corpus entry is a rule nobody will notice regressing.
`every_hard_no_rule_has_a_corpus_entry` enforces that last part rather
than trusting it.

**Refusal frequency is a correctness signal**, not just telemetry. Rare
means the catalogue is drawn right; common means it was drawn too wide,
at which point refusal dialogs train dismissal exactly as Allow dialogs
do. Moving something *out* of HARD NO needs evidence that it fires in
legitimate use, not an argument that it might — hardening after shipping
is much harder than softening after shipping.
