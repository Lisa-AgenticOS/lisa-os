#!/usr/bin/env bash
# Build ONE port — a pinned third-party package under os/packages/ —
# with the flags that package needs (ADR-0051). This is the single
# home of the per-port makepkg recipes: ports.yml builds through it,
# and the aarch64 image lane can adopt it so the flags cannot drift
# between lanes (the two-lists lesson).
#
# Usage: build-port.sh <port-name> <output-dir>
# Runs as an unprivileged user inside an Arch container (makepkg
# refuses root). Debug split packages are filtered from the output.
set -euo pipefail

port=${1:?usage: build-port.sh <port-name> <out-dir>}
out=${2:?usage: build-port.sh <port-name> <out-dir>}

here=$(cd "$(dirname "$0")/../.." && pwd)
dir="$here/os/packages/$port"
[ -d "$dir" ] || { echo "no such port: $dir" >&2; exit 1; }
mkdir -p "$out"

case "$port" in
  llama.cpp)
    # --nocheck: no test target is built (server-only static build).
    (cd "$dir" && makepkg --nocheck --skippgpcheck --noconfirm)
    ;;
  whisper.cpp)
    # NOT --nocheck: check() executes the built binary, which is what
    # caught the build-tree RUNPATH that made an earlier package pass
    # a naive test while being unusable on a real machine.
    (cd "$dir" && makepkg --skippgpcheck --noconfirm)
    ;;
  piper)
    # -s: installs onnxruntime, which it links; espeak-ng is pinned in
    # its own source=() (piper needs an API no espeak-ng release has).
    (cd "$dir" && makepkg -s --skippgpcheck --noconfirm)
    ;;
  lisa-desktop-control-center)
    # -s: arch-meson et al. provides/conflicts=gnome-control-center —
    # stock can never co-install, whatever version Arch ships. Also
    # produces the lisa-desktop-keybindings split package.
    (cd "$dir" && makepkg -s --nocheck --skippgpcheck --noconfirm)
    ;;
  lisa-desktop-online-accounts)
    # Stock GOA with Lisa's own verified Google OAuth client baked in
    # via two meson -D flags (no patch) — the consent screen names the
    # OS that is actually asking. provides/conflicts=
    # gnome-online-accounts, same replacement pattern as the rest of
    # the lisa-desktop-* family.
    (cd "$dir" && makepkg -s --nocheck --skippgpcheck --noconfirm)
    ;;
  *)
    echo "unknown port: $port" >&2
    echo "known: llama.cpp whisper.cpp piper lisa-desktop-control-center lisa-desktop-online-accounts" >&2
    exit 1
    ;;
esac

# shellcheck disable=SC2012
ls "$dir"/*.pkg.tar.* | grep -v -- '-debug-' | while IFS= read -r f; do
  cp "$f" "$out/"
  echo ">> built $(basename "$f")"
done
