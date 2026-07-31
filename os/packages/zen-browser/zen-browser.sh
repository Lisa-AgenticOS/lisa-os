#!/bin/sh
# /usr/bin/zen-browser — resolve the Zen tree, wherever this release keeps
# it (ADR-0023 phase 1, issue #51).
#
# Zen used to be baked into the image at /opt/zen; it now arrives on the
# ADR-0020 apps channel and lives on the persistent /var. This indirection
# is what makes that migration survivable: it is shipped by the tiny
# `zen-browser-launcher` package, which STAYS in the image forever, so the
# .desktop entry and the `zen-browser` command never disappear from under
# the user regardless of which side of the migration a given root slot is on.
#
# Resolution order — first hit wins:
#   $LISA_ZEN_DIR                          (tests, manual overrides)
#   /var/lib/lisa/apps/payloads/zen/current (the apps channel — the future)
#   /opt/zen                                (baked by pre-migration images)
#
# Shell script as a launcher hook only (CLAUDE.md rule 4) — all logic lives
# in `lisa apps` (Rust).
set -eu

for base in "${LISA_ZEN_DIR:-}" \
            /var/lib/lisa-apps/payloads/zen/current \
            /var/lib/lisa/apps/payloads/zen/current \
            /opt/zen; do
    if [ -n "$base" ] && [ -x "$base/zen" ]; then
        exec "$base/zen" "$@"
    fi
done

# Nothing resolved. This is reachable exactly once in a device's life — an
# image without /opt/zen booted before the channel delivered the payload
# (offline install, or an update that could not pre-fetch) — so say what to
# do rather than dying with "command not found". Launched from the app grid
# there is no terminal to read, hence the desktop notification.
msg="Zen is not installed on this system yet.

The browser now arrives through the Lisa app channel instead of the OS
image. Connect to a network and run:

    sudo lisa apps sync

It is also fetched automatically once the machine is online:
    systemctl status lisa-apps-sync.service"

printf '%s\n' "$msg" >&2
if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] && command -v notify-send >/dev/null 2>&1; then
    notify-send --app-name="Zen Browser" --urgency=critical \
        "Zen is not installed yet" \
        "Run 'sudo lisa apps sync' — the browser is fetched from the Lisa app channel." || true
fi
exit 127
