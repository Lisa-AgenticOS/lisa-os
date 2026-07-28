# lisa — the command center CLI

Spec: docs/PLAN.md §5.4 (scriptability), Appendix E rule 4 — read it before changing this component (CLAUDE.md rule 1).

Everything under lisa <verb>: ask (pipes are context), models, and — with the Agent Bus in M5 — tools/call/undo/ledger. One command center, tab completion, no scattered helper scripts.

**M0 state:** ask streams from the OpenAI-compat endpoint (stdin piping works: git log | lisa ask "changelog"); models list/verify/gc/rm/pull against the local store (rm prompts before removing; data reclaimed only by explicit gc). M5 verbs fail loudly with their milestone pointer.

**Forge verbs (PLAN §5.12.1, ADR-0027):** `lisa forge --setup` installs the
pinned Flutter SDK to `/var/lib/lisa/flutter` (sha256-pinned tarball on
x86_64, commit-pinned checkout on aarch64); `lisa forge --flutter "…"`
scaffolds a lisa_ui app and runs the loop against `flutter analyze`;
`lisa forge --build` / `--run` build it for Linux, install the bundle under
the forge apps dir on /var and write the `.desktop` entry that puts it in
the app grid.

**Agent Bus verbs (PLAN §5.4, ADR-0013):** `lisa tools` lists what apps
registered; `lisa call` invokes one directly; `lisa do "<plain words>"`
routes one utterance to exactly one tool call; `lisa undo` reverses the
last undoable one.

**`lisa assist "<plain words>"` (ADR-0025, issue #59)** is the same tools
under the *agent harness* rather than the single-shot router: it can
search, read what came back, decide it needs something else, and search
again, up to `--max-turns` (default 12). Every call is ledgered, and the
run refuses to start if the Ledger cannot be opened (#129).

It offers **read-tier tools only.** Write and destructive tools are
withheld while issue #55 is open: the process hosting the model is also
the process that raises the confirmation dialog, so for a call it
originates itself, requester and approver are the same peer — a
confirmation there is the model asking itself for permission. If a
privileged call does reach the bus, `lisa-agentd` parks it and the loop
reports back that a person has to act; **it never answers its own
confirmation.** The read-tier filter in `bus_tools.rs` is a product
decision about what to advertise, not the guardrail — agentd resolves the
tier from the manifest, on the far side of a call the model cannot forge
(ADR-0030).

**The shell tool (ADR-0036 §6).** `lisa assist` also carries `run_shell`,
for the long tail the typed tools will never cover — pipes, globs,
redirection. Without it the model does what a person would: tell you to
paste the command yourself, which is the same action with none of the
checks and none of the Ledger.

Four conditions, none of them written in a prompt:

1. **Jailed** — it runs with the project directory as its cwd.
2. **Guard-checked** — every line goes through `lisa-guard` first, and a
   refusal never reaches you: being asked about something already refused
   is how people learn to click through prompts.
3. **Never Silent** — even an `Allow` verdict asks. A shell line's real
   tier is not knowable from its text; `curl … | sh` is not a read.
4. **Never unattended** — `ShellTool::new` takes a consent callback and
   there is no other constructor, so a caller *cannot* build one that
   runs without asking. An invariant a caller can forget is not an
   invariant.

The limit worth stating: the jail bounds where the shell *starts*, not
what it can name. A command can still reach outside it. Containing that
needs Landlock (ADR-0029 phase 3), and until then condition 3 is what
stands between the model's idea and your filesystem — which is exactly
why it is not skipped for "obviously safe" commands.

**Skills (ADR-0025):** `lisa skills list` prints the catalog (one
`name: description` line each — the part a prompt carries), `lisa skills
show <name>` prints the workflow body. Resolution: `$LISA_SKILLS_DIR` →
`~/.local/share/lisa/skills` → `/usr/share/lisa/skills`.

**Terminal verbs (PLAN §5.8):** `lisa explain [--exit N] [command…]` explains a failed command (args, piped output, or what the shell hooks stashed) in a few plain streamed sentences; `lisa suggest "<plain words>"` turns natural language into ONE shell command via guided generation and only *prints* it — stdout is exactly the command, the explanation goes to stderr, `--json` emits `{command, explanation}`. The opt-in hooks (error hint, Ctrl+G review gate) live in `apps/terminal-integration/`.
