# Terminal integration

Spec: docs/PLAN.md §5.8 ("Terminal" bullet). Milestone: M6.

The `lisa` CLI is preinstalled (lisa-cli); this component adds the two
in-terminal conveniences the PLAN names — both **opt-in**, both thin
shell hooks around CLI verbs. All logic lives in the Rust CLI
(`cli/lisa/src/terminal.rs`); the hooks only wire keys and prompts to it
(CLAUDE.md rule 4: shell script only in installers and hooks).

## The verbs (work everywhere, hooks or not)

- **`lisa explain [--exit N] [command…]`** — explains a failed command
  and the likely fix in a few plain sentences, streamed from the local
  model at interactive priority. Three input modes:
  - `lisa explain --exit 101 cargo build` — name the command + code;
  - `make 2>&1 | lisa explain` — pipe the output (the tail is excerpted);
  - bare `lisa explain` — uses the last failure the hooks stashed in
    `LISA_LAST_COMMAND` / `LISA_LAST_EXIT`.
- **`lisa suggest "<plain words>"`** — natural language → **one** shell
  command via guided generation (the reply is grammar-constrained to
  `{command, explanation}`). It **never executes anything**: stdout is
  exactly the command (substitutable), the one-line explanation goes to
  stderr (dimmed on a terminal). `--json` emits the raw
  `{command, explanation}` object for scripts.

## The hooks

Shipped to `/usr/share/lisa/terminal/` by the lisa-cli package:

| File | Shell | Mechanics |
|---|---|---|
| `lisa-terminal.zsh` | zsh | ZLE widget bound to `^G` (`bindkey '^G' _lisa_suggest_widget`); `precmd` hook for the error hint |
| `lisa-terminal.bash` | bash | `bind -x '"\C-g": _lisa_suggest_line'` rewriting `READLINE_LINE`; `PROMPT_COMMAND` (prepended) for the error hint |
| `lisa-terminal.sh` | any | the `/etc/profile.d` gate — inert unless `LISA_TERMINAL=1` |

**Ctrl+G — the review gate.** Type what you want in plain words at the
prompt, press Ctrl+G: the line is *replaced* by the suggested command.
Nothing is executed — you read the command sitting in your own prompt
and press Enter yourself. That human Enter, after the swap, **is** the
mandatory review-before-run gate PLAN §5.8 requires; there is no code
path from suggestion to execution.

**Error hint** (extra opt-in: `export LISA_TERMINAL_EXPLAIN=1`). After a
command exits non-zero (Ctrl+C's 130 excepted), one dim line prints:

    ↳ lisa explain — describe this failure

It is a printed hint only — no model call, no latency, nothing runs
until you type `lisa explain`. The hook also stashes the failing command
line and exit code in `LISA_LAST_COMMAND`/`LISA_LAST_EXIT` so the bare
verb has something to explain.

## Enabling (nothing is on by default)

Either source the hook from your shell rc (works for every shell start):

    # ~/.zshrc
    source /usr/share/lisa/terminal/lisa-terminal.zsh
    # ~/.bashrc
    source /usr/share/lisa/terminal/lisa-terminal.bash

or set `LISA_TERMINAL=1` (e.g. in `~/.profile`) and let the packaged
`/etc/profile.d/lisa-terminal.sh` gate source the right file in login
shells. Add `export LISA_TERMINAL_EXPLAIN=1` for the error hint.

## Development

Rust unit tests (prompt composition, guided-generation schema/grammar,
`zsh -n`/`bash -n`/`sh -n` syntax checks on the hooks) live in
`cli/lisa/src/terminal.rs`: `cargo test -p lisa`. Packaging is the
lisa-cli split in `os/packages/lisa/PKGBUILD`.
