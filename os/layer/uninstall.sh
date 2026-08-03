#!/usr/bin/env bash
# Lisa Layer uninstaller (Track L). Reverses every install.sh step and
# leaves a vanilla system. Interactive by default; LISA_ASSUME_YES=1
# skips prompts for CI. User data (/var/lib/lisa: models, state) is
# NEVER removed by this script — it prints where it lives and how to
# remove it deliberately.
set -euo pipefail

say() { printf '%s\n' "$*"; }
die() { say "error: $*" >&2; exit 1; }

confirm() {
    [ "${LISA_ASSUME_YES:-0}" = "1" ] && return 0
    printf '%s [y/N] ' "$1"
    read -r answer
    case "$answer" in y|Y|yes) return 0 ;; *) return 1 ;; esac
}

[ -f /etc/arch-release ] || die "the Lisa Layer targets Arch Linux."
[ "$(id -u)" -eq 0 ] || die "run as root: sudo $0"

if [ -d /run/systemd/system ] && systemctl list-unit-files lisa-inferenced.service >/dev/null 2>&1; then
    if confirm "Stop and disable lisa-inferenced.service?"; then
        systemctl disable --now lisa-inferenced.service 2>/dev/null || true
    fi
fi

installed=$(pacman -Qq lisa-inferenced lisa-modeld lisa-cli lisa-keyring 2>/dev/null || true)
if [ -n "$installed" ]; then
    if confirm "Remove packages: $(echo "$installed" | tr '\n' ' ')?"; then
        # shellcheck disable=SC2086
        pacman -Rns --noconfirm $installed
        # The lsigned trust anchor came from install.sh, so leaving it
        # would keep trusting [lisa] signatures after the layer is gone
        # (#188). Best effort: a file:// install never imported it.
        pacman-key --delete 737240D11D28E109A474A8E5827E27417AF5982B 2>/dev/null || true
    fi
else
    say ">> no lisa packages installed"
fi

if grep -q '^# BEGIN lisa layer' /etc/pacman.conf; then
    if confirm "Remove the [lisa] repo stanza from /etc/pacman.conf?"; then
        sed -i '/^# BEGIN lisa layer/,/^# END lisa layer/d' /etc/pacman.conf
        # Trim a trailing blank line left by removal, if any.
        sed -i -e :a -e '/^\n*$/{$d;N;ba' -e '}' /etc/pacman.conf
    fi
fi

say ""
say "Lisa Layer removed."
if [ -d /var/lib/lisa ]; then
    say "Kept (yours, remove deliberately if wanted): /var/lib/lisa"
fi
