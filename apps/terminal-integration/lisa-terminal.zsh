# lisa-terminal.zsh — Lisa terminal integration for zsh (PLAN §5.8).
#
# Wiring only: every decision lives in the `lisa` CLI (CLAUDE.md rule 4).
# Opt in by sourcing this from ~/.zshrc:
#
#     source /usr/share/lisa/terminal/lisa-terminal.zsh
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

[[ -o interactive ]] || return 0

# --- error hint on non-zero exit (opt-in) ----------------------------------
_lisa_explain_hint() {
    local code=$?    # must be the first statement: the failed command's status
    [[ -n "$LISA_TERMINAL_EXPLAIN" ]] || return 0
    (( code != 0 )) || return 0
    (( code == 130 )) && return 0    # Ctrl+C is not a failure to explain
    # Stash the facts so a bare `lisa explain` knows the last failure.
    export LISA_LAST_EXIT=$code
    export LISA_LAST_COMMAND="$(fc -ln -1 2>/dev/null)"
    printf '\033[2m↳ lisa explain — describe this failure\033[0m\n' >&2
    return 0
}
autoload -Uz add-zsh-hook
add-zsh-hook precmd _lisa_explain_hint

# --- Ctrl+G: natural language -> command (review gate) ---------------------
_lisa_suggest_widget() {
    if [[ -z "$BUFFER" ]]; then
        zle -M "lisa: type what you want in plain words, then press Ctrl+G"
        return 0
    fi
    zle -M "lisa: suggesting…"
    zle -R
    local tmp cmd expl rc
    tmp="$(mktemp)" || return 1
    # stdout is exactly the command; the explanation arrives on stderr.
    cmd="$(lisa suggest -- "$BUFFER" 2>"$tmp")"
    rc=$?
    expl="$(<"$tmp")"
    rm -f -- "$tmp"
    if (( rc != 0 )) || [[ -z "$cmd" ]]; then
        zle -M "lisa suggest failed${expl:+: $expl}"
        return 1
    fi
    # Replace the buffer — NEVER execute. The user reviews the command
    # and presses Enter themselves (the mandatory review gate).
    BUFFER="$cmd"
    CURSOR=${#BUFFER}
    zle -M "${expl:-review the command, then press Enter to run it}"
    zle redisplay
}
zle -N _lisa_suggest_widget
bindkey '^G' _lisa_suggest_widget
