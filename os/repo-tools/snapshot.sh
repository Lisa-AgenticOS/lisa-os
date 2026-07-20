#!/usr/bin/env bash
# Pinned Arch snapshot management (PLAN §3 "packaging economics", §6).
# The base moves only when we say so, like SteamOS's holo repo: image and
# layer builds resolve Arch packages from the Arch Linux Archive snapshot
# recorded in snapshot.txt, advanced deliberately at channel promotion.
#
#   snapshot.sh show              print the current pin and its mirror URL
#   snapshot.sh set YYYY/MM/DD    verify the archive snapshot exists, pin it
#   snapshot.sh latest            pin yesterday's snapshot (always complete)
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
pin_file="$here/snapshot.txt"
archive="https://archive.archlinux.org/repos"

mirror_url() { printf '%s/%s' "$archive" "$1"; }

verify() {
    # A usable snapshot serves core's repo database.
    local probe
    probe="$(mirror_url "$1")/core/os/x86_64/core.db"
    curl -sfIL --max-time 30 "$probe" >/dev/null || {
        echo "error: no Arch archive snapshot at $1 (probe failed: $probe)" >&2
        return 1
    }
}

case "${1:-show}" in
show)
    [ -f "$pin_file" ] || { echo "no snapshot pinned yet — run: $0 set YYYY/MM/DD" >&2; exit 1; }
    pin=$(cat "$pin_file")
    echo "pinned:  $pin"
    echo "mirror:  $(mirror_url "$pin")"
    ;;
set)
    pin=${2:?usage: $0 set YYYY/MM/DD}
    [[ "$pin" =~ ^[0-9]{4}/[0-9]{2}/[0-9]{2}$ ]] || { echo "error: format is YYYY/MM/DD" >&2; exit 1; }
    verify "$pin"
    printf '%s\n' "$pin" >"$pin_file"
    echo "pinned $pin"
    ;;
latest)
    # Yesterday, not today: today's snapshot may still be syncing.
    pin=$(date -u -d yesterday +%Y/%m/%d 2>/dev/null || date -u -v-1d +%Y/%m/%d)
    verify "$pin"
    printf '%s\n' "$pin" >"$pin_file"
    echo "pinned $pin"
    ;;
*)
    echo "usage: $0 {show|set YYYY/MM/DD|latest}" >&2
    exit 2
    ;;
esac
