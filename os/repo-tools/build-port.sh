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
  zen-browser)
    # -d: binary repackage of the upstream tarball (sha256-pinned in
    # the PKGBUILD); runtime deps resolve at image-install time.
    # Produces BOTH split packages (zen-browser + zen-browser-launcher).
    (cd "$dir" && makepkg -d --nocheck --noconfirm)
    ;;
  gnome-control-center-lisa)
    # -s: arch-meson et al. Takes the gnome-control-center name so
    # repo precedence installs it over stock. Also produces the
    # gnome-keybindings split package.
    (cd "$dir" && makepkg -s --nocheck --skippgpcheck --noconfirm)
    ;;
  gnome-online-accounts-lisa)
    # Stock GOA with Lisa's own verified Google OAuth client baked in
    # via two meson -D flags (no patch) — the consent screen names the
    # OS that is actually asking. Takes the gnome-online-accounts name
    # for repo precedence, like the g-c-c fork above.
    (cd "$dir" && makepkg -s --nocheck --skippgpcheck --noconfirm)
    ;;
  *)
    echo "unknown port: $port" >&2
    echo "known: llama.cpp whisper.cpp piper zen-browser gnome-control-center-lisa gnome-online-accounts-lisa" >&2
    exit 1
    ;;
esac

# shellcheck disable=SC2012
ls "$dir"/*.pkg.tar.* | grep -v -- '-debug-' | while IFS= read -r f; do
  cp "$f" "$out/"
  echo ">> built $(basename "$f")"
done
