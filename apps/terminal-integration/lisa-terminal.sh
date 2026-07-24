# /etc/profile.d/lisa-terminal.sh — opt-in gate for Lisa's terminal
# integration (PLAN §5.8). Does nothing — prints nothing, sources
# nothing — unless the user opted in with:
#
#     export LISA_TERMINAL=1
#
# profile.d only reaches login shells; the always-reliable route is one
# line in your shell rc instead (see README in apps/terminal-integration):
#     zsh:   source /usr/share/lisa/terminal/lisa-terminal.zsh
#     bash:  source /usr/share/lisa/terminal/lisa-terminal.bash
if [ -n "$LISA_TERMINAL" ]; then
    if [ -n "$ZSH_VERSION" ] && [ -r /usr/share/lisa/terminal/lisa-terminal.zsh ]; then
        . /usr/share/lisa/terminal/lisa-terminal.zsh
    elif [ -n "$BASH_VERSION" ] && [ -r /usr/share/lisa/terminal/lisa-terminal.bash ]; then
        . /usr/share/lisa/terminal/lisa-terminal.bash
    fi
fi
