# lisa-terminal.bash — Lisa terminal integration for bash (PLAN §5.8).
#
# Wiring only: every decision lives in the `lisa` CLI (CLAUDE.md rule 4).
# Opt in by sourcing this from ~/.bashrc:
#
#     source /usr/share/lisa/terminal/lisa-terminal.bash
#
# (or export LISA_TERMINAL=1 and let /etc/profile.d/lisa-terminal.sh
# source it in login shells — see README.md).
#
# What you get:
#   * Ctrl+G — type what you want in plain words, press Ctrl+G: the line
#     is REPLACED by one suggested shell command. It is never run for
#     you; you read it and press Enter yourself. That is the mandatory
#     review-before-run gate.
#   * Error hint (extra opt-in: export LISA_TERMINAL_EXPLAIN=1) — after
#     a command fails, one dim line reminds you that `lisa explain` can
#     describe the failure. A printed hint only: no model call, no
#     latency, nothing leaves the machine.

[[ $- == *i* ]] || return 0

# --- error hint on non-zero exit (opt-in) ----------------------------------
_lisa_explain_hint() {
    local code=$?    # must be the first statement: the failed command's status
    [[ -n "$LISA_TERMINAL_EXPLAIN" ]] || return 0
    [[ $code -ne 0 ]] || return 0
    [[ $code -ne 130 ]] || return 0    # Ctrl+C is not a failure to explain
    # Stash the facts so a bare `lisa explain` knows the last failure.
    export LISA_LAST_EXIT=$code
    export LISA_LAST_COMMAND="$(fc -ln -1 2>/dev/null)"
    printf '\033[2m↳ lisa explain — describe this failure\033[0m\n' >&2
    return 0
}
# Prepend, so $? still belongs to the user's command, not another hook.
PROMPT_COMMAND="_lisa_explain_hint${PROMPT_COMMAND:+;$PROMPT_COMMAND}"

# --- Ctrl+G: natural language -> command (review gate) ---------------------
_lisa_suggest_line() {
    [[ -n "$READLINE_LINE" ]] || return 0
    local tmp cmd expl rc
    tmp="$(mktemp)" || return 1
    # stdout is exactly the command; the explanation arrives on stderr.
    cmd="$(lisa suggest -- "$READLINE_LINE" 2>"$tmp")"
    rc=$?
    expl="$(<"$tmp")"
    rm -f -- "$tmp"
    if [[ $rc -ne 0 || -z "$cmd" ]]; then
        printf 'lisa suggest failed%s\n' "${expl:+: $expl}" >&2
        return 1
    fi
    [[ -n "$expl" ]] && printf '\033[2m%s\033[0m\n' "$expl" >&2
    # Replace the line — NEVER execute. The user reviews the command and
    # presses Enter themselves (the mandatory review gate).
    READLINE_LINE="$cmd"
    READLINE_POINT=${#READLINE_LINE}
    return 0
}
bind -x '"\C-g": _lisa_suggest_line'
