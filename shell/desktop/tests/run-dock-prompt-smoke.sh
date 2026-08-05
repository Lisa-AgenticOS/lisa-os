#!/usr/bin/env bash
#
# Run dock-prompt-smoke.js inside a real, throwaway GNOME Shell
# (ADR-0035 §2, #190).
#
# WHAT IT NEEDS: a Linux host with gnome-shell >= 50, gjs,
# dbus-run-session and a render node. It runs `gnome-shell --headless`,
# which needs no seat, no display and no login session, so it is safe
# beside a live session. On a host without gnome-shell it SKIPS (exit 0)
# and says so — a macOS dev host cannot run this and should not fail for
# it.
#
# ---------------------------------------------------------------------
# THE TRAP — read before edit (it cost an hour and a user's settings once)
# ---------------------------------------------------------------------
# `dbus-run-session` starts the bus BEFORE it execs its child, so the bus
# inherits the environment of whatever invoked it — and dconf is a
# *service on that bus*, resolving $XDG_CONFIG_HOME from the bus daemon's
# environment rather than the child's. Exporting the isolated XDG_*
# inside the child script looks right and silently writes to the REAL
# user's ~/.config/dconf/user.
#
# So every XDG_* override is set on the `env` that wraps
# `dbus-run-session`, never inside the script it runs. The md5 guard
# below is the regression test for this paragraph.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
desktop=$(dirname "$here")

if ! command -v gnome-shell >/dev/null || ! command -v dbus-run-session >/dev/null; then
    echo "SKIP: dock prompt smoke needs gnome-shell + dbus-run-session (Linux only)"
    exit 0
fi

root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT

ext="$root/data/gnome-shell/extensions"
apps="$root/data/applications"
mkdir -p "$ext/lisa-desktop@lisa-os.org/lib" \
         "$ext/lisa-dock-prompt-smoke@lisa-os.org" \
         "$apps" "$root/config" "$root/cache" "$root/state" "$root/run"
chmod 700 "$root/run"

# The extension under test, exactly as it ships.
cp "$desktop"/*.js "$desktop"/*.json "$desktop"/*.css "$desktop"/*.svg \
   "$ext/lisa-desktop@lisa-os.org/" 2>/dev/null || true
cp "$desktop"/lib/*.js "$ext/lisa-desktop@lisa-os.org/lib/"

cp "$here/dock-prompt-smoke.js" "$ext/lisa-dock-prompt-smoke@lisa-os.org/extension.js"
cat > "$ext/lisa-dock-prompt-smoke@lisa-os.org/metadata.json" <<'JSON'
{
  "uuid": "lisa-dock-prompt-smoke@lisa-os.org",
  "name": "Lisa dock prompt smoke",
  "description": "Types into the dock's prompt and badges its icons, then reports what happened.",
  "shell-version": ["45", "46", "47", "48", "49", "50", "51"]
}
JSON

# The app the probe types the name of. `Exec` leaves a file behind, so
# "did it launch" is an observation rather than an inference.
mark="$root/launched"
cat > "$apps/smokeapp.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=smokeapp
Exec=touch $mark
Icon=utilities-terminal
Terminal=false
DESKTOP
# A second one, so changing favourites gives the Dash a real rebuild.
cat > "$apps/smokeapp2.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=smokeapp2
Exec=true
Icon=utilities-terminal
Terminal=false
DESKTOP
update-desktop-database "$apps" 2>/dev/null || true

report="$root/report.txt"
: > "$report"

# The guard for the trap described above.
dconf_db="${XDG_CONFIG_HOME:-$HOME/.config}/dconf/user"
before=$( [ -f "$dconf_db" ] && md5sum "$dconf_db" | cut -d' ' -f1 || echo none )

cat > "$root/session.sh" <<'SESSION'
#!/usr/bin/env bash
# Runs INSIDE the isolated bus. Deliberately sets no XDG_* itself.
gsettings set org.gnome.shell disable-user-extensions false
gsettings set org.gnome.shell enabled-extensions \
    "['lisa-desktop@lisa-os.org', 'lisa-dock-prompt-smoke@lisa-os.org']"
gsettings set org.gnome.shell favorite-apps "['smokeapp.desktop']"
exec gnome-shell --headless --virtual-monitor 1920x1080 \
    --wayland-display "lisa-prompt-smoke-$$" --no-x11
SESSION
chmod +x "$root/session.sh"

echo "== dock prompt smoke: starting a headless gnome-shell"
env -u WAYLAND_DISPLAY -u DISPLAY \
    XDG_DATA_HOME="$root/data" \
    XDG_CONFIG_HOME="$root/config" \
    XDG_CACHE_HOME="$root/cache" \
    XDG_STATE_HOME="$root/state" \
    XDG_RUNTIME_DIR="$root/run" \
    XDG_DATA_DIRS="$root/data:/usr/local/share:/usr/share" \
    LISA_SMOKE_REPORT="$report" \
    LISA_SMOKE_LAUNCH_MARK="$mark" \
    timeout 180 dbus-run-session -- "$root/session.sh" \
    > "$root/shell.log" 2>&1 || true

after=$( [ -f "$dconf_db" ] && md5sum "$dconf_db" | cut -d' ' -f1 || echo none )
if [ "$before" != "$after" ]; then
    echo "FAIL: the smoke run modified the real user's dconf — read the header of this file" >&2
    exit 1
fi

echo "----- transcript -----"
cat "$report"
echo "----------------------"

# Anything the extension logged is a failure, even with every assertion
# green. `logError` inside a signal handler does not stop the shell and
# never reaches the transcript — it lands in this log and nowhere else,
# so a probe that reported only its own assertions would be describing a
# shell it had already broken. `lisa-desktop:` is the prefix every
# logError in the extension carries.
if grep -qE 'lisa-desktop:' "$root/shell.log"; then
    echo "FAIL: the extension logged an error:" >&2
    grep -E -A6 'lisa-desktop:' "$root/shell.log" | head -40 >&2
    exit 1
fi

# No transcript at all means the shell never got far enough to run the
# probe; that is a failure, not a pass. Never infer success from silence.
if ! grep -q '^RESULT: ' "$report"; then
    echo "FAIL: the probe produced no verdict — last 30 lines of the shell log:" >&2
    tail -30 "$root/shell.log" >&2
    exit 1
fi

if grep -q '^RESULT: PASS' "$report"; then
    echo "dock prompt smoke: PASS"
    exit 0
fi

echo "dock prompt smoke: FAIL" >&2
tail -30 "$root/shell.log" >&2
exit 1
