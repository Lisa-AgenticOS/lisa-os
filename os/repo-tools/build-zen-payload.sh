#!/usr/bin/env bash
# Build the apps-channel Zen payload: lisa-zen_<ver>_<arch>.tar.zst
# (ADR-0023 phase 1, issue #51).
#
# The payload is exactly the tree that used to be baked as /opt/zen, so
# `lisa apps` can unpack it to /var/lib/lisa/apps/payloads/zen/versions/<ver>
# and /usr/bin/zen-browser resolves `<tree>/zen` with no path translation.
#
# NO DIGESTS LIVE HERE. The upstream URL and its pinned sha256 are read from
# os/packages/zen-browser/PKGBUILD, which stays the single place a Zen
# version is pinned (CLAUDE.md rule 8) — image and channel therefore ship
# the byte-identical browser, and re-pinning is still a one-file change.
#
# Both architectures build on ANY host: this is a repackage of an upstream
# binary tarball, not a compile, so release.yml produces the aarch64 payload
# on its x86_64 runner and the channel is complete from a single release.
#
# Usage: build-zen-payload.sh <x86_64|aarch64> <version> [outdir]
set -euo pipefail

usage="usage: build-zen-payload.sh <x86_64|aarch64> <version> [outdir]"
# zp_ prefix throughout, deliberately: sourcing a PKGBUILD dumps ITS
# variables into this scope, and `arch=(x86_64 aarch64)` / `url=` would
# quietly overwrite plain `arch`/`url` here — which built the x86_64 payload
# under an aarch64 name until this prefix was added. Do not un-prefix these.
zp_arch=${1:?$usage}
zp_ver=${2:?$usage}
zp_out=${3:-$PWD}

zp_here=$(cd "$(dirname "${BASH_SOURCE[0]}")/../packages/zen-browser" && pwd)
# The PKGBUILD is a bash fragment with no side effects at source time; this
# reads its pkgver/source_*/sha256sums_* rather than duplicating them.
# shellcheck source=/dev/null
source "$zp_here/PKGBUILD"

# pkgver/source_*/sha256sums_* all come from the sourced PKGBUILD above.
# shellcheck disable=SC2154
case "$zp_arch" in
  x86_64)  zp_entry=${source_x86_64[0]};  zp_want=${sha256sums_x86_64[0]} ;;
  aarch64) zp_entry=${source_aarch64[0]}; zp_want=${sha256sums_aarch64[0]} ;;
  *) echo "build-zen-payload.sh: unsupported arch '$zp_arch'" >&2; exit 2 ;;
esac
# makepkg source entries are "<local name>::<url>".
zp_url=${zp_entry#*::}
if [ "$zp_url" = "$zp_entry" ]; then
  echo "build-zen-payload.sh: source entry has no URL: $zp_entry" >&2
  exit 2
fi
if [ -z "$zp_want" ] || [ "$zp_want" = SKIP ]; then
  echo "build-zen-payload.sh: refusing to build an unpinned payload for $zp_arch" >&2
  exit 2
fi

# coreutils on Linux/CI, shasum on the macOS dev host (CLAUDE.md: the dev
# host may be macOS, so the packaging tools have to run there too).
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

zp_work=$(mktemp -d)
trap 'rm -rf "$zp_work"' EXIT

# shellcheck disable=SC2154  # pkgver comes from the sourced PKGBUILD
echo ">> zen $pkgver ($zp_arch) <- $zp_url"
curl -fL --retry 3 --retry-delay 5 -o "$zp_work/zen.tar.xz" "$zp_url"
zp_got=$(sha256_of "$zp_work/zen.tar.xz")
if [ "$zp_got" != "$zp_want" ]; then
  echo "build-zen-payload.sh: sha256 mismatch for $zp_arch" >&2
  echo "  pinned: $zp_want" >&2
  echo "  got:    $zp_got" >&2
  echo "  Re-pin os/packages/zen-browser/PKGBUILD after verifying upstream." >&2
  exit 1
fi

mkdir -p "$zp_work/x"
tar -xJf "$zp_work/zen.tar.xz" -C "$zp_work/x"
# Same shape the image package relies on — a tarball that stopped unpacking
# to zen/zen must fail here, not on a user's desktop.
test -x "$zp_work/x/zen/zen" || {
  echo "build-zen-payload.sh: no executable zen/zen in the tarball" >&2
  exit 1
}

mkdir -p "$zp_out"
zp_dest="$zp_out/lisa-zen_${zp_ver}_${zp_arch}.tar.zst"
# Piped through zstd rather than `tar --zstd` so the LEVEL is ours: at the
# default level this tree packs to 126 MB, at -19 to 100 MB (measured
# 2026-07-26). Every device on the channel pays that difference on every Zen
# update; ~20 s of CI once does not. -f: rebuilds overwrite.
tar -C "$zp_work/x/zen" -cf - . | zstd -q -f -T0 -19 -o "$zp_dest"
echo ">> $zp_dest ($(du -h "$zp_dest" | cut -f1)), tree $(du -sh "$zp_work/x/zen" | cut -f1)"
