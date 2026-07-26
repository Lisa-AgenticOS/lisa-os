# ADR-0029: Hard guardrails for agent actions — policy outside the model

- Status: accepted (phase 1 implemented 2026-07-26)
- Date: 2026-07-26
- Relates: ADR-0025 (one agent loop), PLAN §5.4 (Agent Bus), §5.10 +
  Appendix C (provenance, injection), §5.12.1 (forge tool jail), M5
  acceptance (injection suite), issue #23
- Supersedes in part: the "the same jail confines BYO agent backends"
  claim in `libs/forge-harness/src/jail.rs`, which was true of `..` and
  absolute paths and false of symlinks.
- **Amended by [ADR-0030](0030-the-guardrail-boundary.md):** this ADR
  made `Deny` absolute with no override, which placed a guardrail between
  the owner and their own machine. It is now relaxable by the human,
  out-of-band, via `lisa guard allow` — and still unreachable from inside
  the probabilistic system, which is the property that actually mattered.
  Read ADR-0030 for where the boundary falls and the test for it.

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

A check is not a guarantee, though, and the first review round measured
exactly how much not: against a check-then-`fs::write` jail, a thread
swapping the target for an outside symlink landed **18,599 of 20,001
writes outside the root**. So writes do not go through `fs::write` at
all — `write_contained` opens with `O_NOFOLLOW`, which makes the kernel
refuse rather than trusting what we saw a moment earlier.

`O_NOFOLLOW` guards the final component only. Swapping a *parent*
directory between the check and the open remains possible; that needs
`openat2(RESOLVE_BENEATH)` or Landlock, and is tracked with phase 3
rather than papered over. Reads still rely on the check alone, which is
a disclosure race, not a write race.

This is the rule the user named: **the agent has access to the directory
it was spawned in, and nothing above it.**

### 2. Command policy: allowlist the program, then police the argv

Program allowlisting stays, but it is no longer the whole story, because
an allowlisted program with an exec predicate *is* a shell. Three layers,
all deterministic:

- **Program allowlist** (`ALLOWED_COMMANDS`, moved into the guard so one
  crate owns policy). A program is a bare name; naming one by path would
  sidestep both the allowlist and every rule that matches on the program.
- **Per-program argument policy.** Each allowlisted program declares
  which flags are refused, which subcommands it may take, and which of
  its arguments are patterns rather than paths. `find` loses `-exec`,
  `-execdir`, `-ok`, `-okdir`, `-delete`, `-fprintf`, `-fprint`,
  `-fprint0`, `-fls`; `cargo` loses `--config` and any unrecognised
  subcommand. This closes each pivot at its source rather than trying to
  recognise the payload.
- **Destructive-pattern scan** over the rendered invocation, shared with
  the shell-line path below, as the backstop for anything the first two
  layers let through.

The generalization from "a denied-flag table for `find`" to "a policy per
program" is the first review round's doing, and its reasoning is worth
keeping: **an allowlisted program is only as narrow as its own flag
surface.** `cargo --config 'target."cfg(all())".runner=["/bin/sh",…]'`
is `find -exec` wearing a build tool, and an unknown `cargo <verb>`
resolves to whatever `cargo-<verb>` sits on `PATH`. `rustc` left the
allowlist entirely for the same reason — a raw compiler invocation can
emit a binary anywhere, and the loop only ever needs `cargo`.

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

The shell-line analyzer splits on `;`, `&&`, `||`, `|`, on subshells and
brace groups, unwraps `sudo`/`doas`/`env`/`busybox` prefixes and shell
keywords, reduces each program to its basename, recurses into command
substitution, `eval`, and `sh -c` scripts, and runs the same rules per
segment. A `Deny` suppresses the suggestion and says why; a `Confirm`
still pre-types it but prints the reason first, so the user's Enter is
informed rather than reflexive.

Its governing rule, added after the first review round: **fail closed on
anything it cannot model.** This is a conservative *reader* of a shell,
not a shell, and the gap between those two is where `rm${IFS}-rf${IFS}/`
and `$'\x72\x6d' -rf /` lived. Expansions it understands are normalized
first; a program name still computed at runtime (`${CMD} -rf /`) is
refused outright rather than guessed at.

## Review round 1 (2026-07-26, same day)

Phase 1 shipped with a green corpus and an adversarial review found
**eight bypasses**, every one demonstrated by execution rather than
argued (issues #59–#66). They are recorded here because the pattern
matters more than the individual bugs:

| # | what got through | why |
|---|---|---|
| 59 | `/bin/rm -rf /`, `busybox rm -rf /` | every rule matched a bare basename |
| 60 | `rm${IFS}-rf${IFS}/`, `$'\x72\x6d' -rf /` | the reader modelled no expansion at all |
| 61 | `( rm -rf / )`, `{ rm -rf /; }`, `eval`, `f(){…}`, `<(…)` | compound commands were never split |
| 62 | `/usr/../etc`, `//etc` | target normalization was suffix-stripping, not path-aware |
| 63 | `cargo test --config '…runner=["/bin/sh",…]'` | `find -exec` again, through the build tool |
| 64 | `grep -f/etc/passwd` | attached short-option values were never inspected |
| 65 | `grep /etc/passwd src` refused | the same scan was also too broad |
| 66 | 18,599/20,001 writes escaped | check-then-write is a race a symlink swap wins |

Three conclusions, all now load-bearing:

1. **The corpus proves the spellings it contains and nothing else.** It
   was green because it listed each attack in its plainest form. Every
   line above is in it now, and new rules must arrive with a corpus entry.
2. **A conservative reader must fail closed.** #60 and #61 were not
   missing rules; they were a parser quietly returning "nothing to see"
   for input it could not read. `shell.unreadable` exists for that.
3. **The ADR and the code disagreed, and the ADR was the optimistic
   one.** §1 claimed writes refuse a symlink target; nothing implemented
   it. Design text written before the code has to be re-read against the
   code afterwards, which is the same lesson ADR-0023 learned about
   estimates versus measurements.

#65 is worth naming separately: it is a *false positive*, and it was
filed alongside the escapes deliberately. A guard that blocks
`grep /etc/passwd src` is a guard people route around, and a routed-around
guard protects nothing.

## Review round 2 (2026-07-26)

The round-1 fixes were reviewed the same way and **eleven more findings**
came back (#67–#77). Every one of them is the round-1 fix being correct
for exactly the spelling that prompted it:

| # | what got through | the round-1 fix it sat next to |
|---|---|---|
| 67 | `env -u FOO rm -rf /`, `timeout -s KILL 5 rm …` | wrappers were unwrapped, but their *options taking a separate value* made the value look like the program |
| 68 | `&>`, `&>>`, `>\|`, `\|&` | redirects were judged, but not in these spellings; a word-less segment skipped its redirect entirely |
| 69 | `> //etc/passwd`, `dd of=//dev/sda` | `normalize_target` was written for #62 and never wired into the redirect or `dd` paths |
| 70 | `bash -xc`, `sh <<<`, `trap '…' EXIT` | `sh -c` was read — only in that exact spelling |
| 71 | `python3 -c 'os.system("rm -rf /")'` | read *as shell*, which it is not, so it parsed to nothing a rule matched |
| 72 | `grep -e foo /etc/passwd` | the #64/#65 position-awareness left the pattern slot open, so the path was eaten as the pattern |
| 73 | `rm -rf /etc/systemd/system` | `is_system_target` capped at depth 2; the depth-independent check existed and was used only by the writer rules |
| 74 | `dart pub global activate` | subcommand allowlists stopped at the first level |
| 75 | `cp /etc/os-release .` refused | source operands read as destinations |
| 76 | `cargo +nightly fmt` refused | `+toolchain` and separate flag values read as the subcommand |
| 77 | `rm -rf $TARGET` | a runtime *program name* was refused; a runtime *target* was not |

The recurring shape is now explicit, and it is the reason three of these
fixes changed strategy rather than adding a case:

> **Enumerating the dangerous spellings of a shell is a losing game.**

So where the reader kept leaking, it stopped enumerating:

- **Wrappers** no longer have their option grammars modelled. If a
  wrapper is present, *every* subsequent word is judged as a candidate
  program. Cheap, and it errs toward seeing more commands than fewer.
- **Inline source** is no longer matched flag by flag. Every literal
  argument of a shell is read as a command line, and a language this
  reader does not speak (`python -c`, `perl -e`, `node -e`, `php -r`) is
  **refused outright** rather than approximated — #71 is precisely what
  approximation buys.
- **Unresolved words** are refused in target position as well as program
  position. `rm -rf $TARGET` is the same unknown as `${CMD} -rf /`.

Corpus: 75 → 105 denied entries, 17 → 29 must-allow. The must-allow half
grew deliberately: rounds 1 and 2 each produced a false positive
(#65, #75/#76), and those are as load-bearing as the escapes.

**Two rounds, nineteen findings, in code whose entire claim is that it
cannot be talked past.** That is the honest character of this surface,
and it is why the corpus is described as a floor everywhere it appears.
`check_shell_line` guards a *suggestion* — the blast radius of a miss is
one command a human still has to press Enter on — while `check_command`
guards autonomous execution and is a far smaller, argv-shaped problem.
If the shell reader needs a third strategy change, the right answer is
probably to stop reading shell at all and have `lisa suggest` emit
structured argv the guard can judge exactly.

## Review round 3 (2026-07-26) — and the decision it forces

Ten more (#78–#87). Findings per round: **8 → 11 → 10**. Flat, not
converging, and the composition is worse than the count:

- **#85 is a regression this ADR's own round-2 fix created.** `cargo --
  evil-plugin` runs `cargo-evil-plugin` from `PATH`. The `--` split was
  added to stop a false positive (#76) and skipped every argument after
  `--` — but `--` only delimits an inner program's argv *once a
  subcommand has been named*. A fix for over-refusal became arbitrary
  program execution on the surface that runs with no human, no ledger and
  no confinement. It was fixed and landed on its own, ahead of the rest.
- **Four findings were new classes** nothing in rounds 1–2 anticipated:
  the round-1 decoder feeding a quote to the round-1 tokenizer (#78), an
  unmodelled operator (#82), an unmodelled metacharacter class — globs in
  target position (#83), and an unmodelled comment (#87).
- **Two of round 2's three "stop enumerating" changes were still closed
  lists.** `WRAPPERS` and `SHELLS` are enumerations, and `flock` and
  `python3.11` walked past them (#81, #80).

So round 3 stopped enumerating one level further up: a program this crate
**does not recognise at all** is now treated as possibly-a-wrapper, and
its words are judged as candidate programs. And the shell/interpreter
split was made explicit — only a real shell's arguments are read *as
shell*, because reading Python that way produced #71 and reading awk that
way denied `awk '{print $1}'`.

Corpus: 105 → 128 denied, 29 → 41 must-allow.

### Decision: `lisa suggest` moves to structured argv

`check_shell_line` is not converging, and the two surfaces this crate
guards have very different shapes:

| | input | leaks |
|---|---|---|
| `check_command` | argv, small and bounded | 4 across 3 rounds, 1 self-inflicted |
| `check_shell_line` | an arbitrary shell string | 15 across 3 rounds, still finding new classes |

The difference is the input, not the effort. So the fallback named
earlier in this ADR is now the plan (tracked as an issue): **have `lisa
suggest` emit structured argv** — `{program, args, operators}` — judge it
with `check_command`-shaped logic, and render a shell string only for
display. That deletes #78, #79, #82, #83, #87 and every future member of
those families outright, because there is nothing to tokenize. The cost
is bounded and one-time: pipelines and redirects need an explicit
representation, and some suggestions become several steps instead of one
line. The current cost is one adversarial round per release, forever.

`check_shell_line` stays as defence-in-depth for strings arriving from
elsewhere, and should have its default flipped as part of that work:
unknown operator, unbalanced quote after decoding, or an unresolved word
in target position become `shell.unreadable` rather than `Allow`. That is
round 1's conclusion — a conservative reader must fail closed — applied
to the *parser* rather than only to the program name.

**The through-line of all three rounds:** every fix was correct, and
every fix left the next spelling open. That is what a guardrail built by
enumeration looks like from the inside, and it is why the corpus is
called a floor everywhere it appears — it was green after every round.

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
