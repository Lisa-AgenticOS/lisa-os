#!/usr/bin/env bash
# Lisa Layer installer (Track L, ADR-0003): the Lisa stack on stock Arch
# or Omarchy — no custom image required. Root required (pacman).
#
# Repo source: the hosted, SIGNED [lisa] index by default (ADR-0039,
# ADR-0041) — live since 2026-08-03. Set LISA_REPO_URL to override with
# a local build (os/repo-tools/build-packages.sh); local repos stay
# SigLevel Optional, the hosted one is verified against the lisa-keyring
# trust anchor and ends at SigLevel Required.
set -euo pipefail

say() { printf '%s\n' "$*"; }
die() { say "error: $*" >&2; exit 1; }

[ -f /etc/arch-release ] || die "the Lisa Layer targets Arch Linux (including Omarchy)."
[ "$(id -u)" -eq 0 ] || die "run as root (pacman needs it): sudo $0"
HOSTED_REPO="https://github.com/Lisa-AgenticOS/lisa-packages/releases/download/current"
LISA_REPO_URL="${LISA_REPO_URL:-$HOSTED_REPO}"

HOSTED_KEY_URL="https://raw.githubusercontent.com/Lisa-AgenticOS/lisa-packages/main/lisa-packages.gpg.asc"
LISA_FPR="737240D11D28E109A474A8E5827E27417AF5982B"

if [ "$LISA_REPO_URL" = "$HOSTED_REPO" ]; then
    # Trust FIRST, repo second: the signing key arrives over the same
    # TLS-to-GitHub the packages would anyway, gets locally signed
    # against the pinned fingerprint, and the stanza starts at
    # Required — no package is ever installed unverified. (SigLevel
    # Optional would not have helped bootstrap: it forgives a MISSING
    # signature, not one it cannot verify, and ours exist.)
    say ">> importing the [lisa] signing key (fingerprint $LISA_FPR)"
    # Unconditional: --init is idempotent, and a gnupg dir EXISTING is
    # not the same as one holding the master secret key --lsign-key
    # signs with (a half-initialized dir fails exactly there).
    pacman-key --init
    tmpkey=$(mktemp)
    curl -fsSL "$HOSTED_KEY_URL" -o "$tmpkey"
    pacman-key --add "$tmpkey"
    rm -f "$tmpkey"
    pacman-key --lsign-key "$LISA_FPR"
    SIGLEVEL="Required"
else
    # A local build (file://) has no signatures; say so in the stanza.
    SIGLEVEL="Optional TrustAll"
fi

if grep -q '^\[lisa\]' /etc/pacman.conf; then
    say ">> [lisa] repo already configured, leaving pacman.conf untouched"
else
    say ">> adding [lisa] repo to /etc/pacman.conf (SigLevel = $SIGLEVEL)"
    cat >>/etc/pacman.conf <<EOF

# BEGIN lisa layer (added by os/layer/install.sh; removed by uninstall.sh)
[lisa]
SigLevel = $SIGLEVEL
Server = $LISA_REPO_URL
# END lisa layer
EOF
fi

say ">> installing lisa-inferenced lisa-modeld lisa-cli (full -Syu: partial upgrades break Arch)"
EXTRA=""
[ "$LISA_REPO_URL" = "$HOSTED_REPO" ] && EXTRA="lisa-keyring"
pacman -Syu --noconfirm lisa-inferenced lisa-modeld lisa-cli $EXTRA

if [ -d /run/systemd/system ]; then
    say ">> enabling lisa-inferenced.service"
    systemctl daemon-reload
    systemctl enable --now lisa-inferenced.service
    say ">> waiting for the inference endpoint"
    # LISA_HEALTH_TIMEOUT: seconds to wait (default 10; CI containers are slow).
    for _ in $(seq 1 $((5 * ${LISA_HEALTH_TIMEOUT:-10}))); do
        curl -sf 127.0.0.1:7777/health >/dev/null 2>&1 && break
        sleep 0.2
    done
    curl -sf 127.0.0.1:7777/health >/dev/null || die "lisa-inferenced did not come up; see: journalctl -u lisa-inferenced"
else
    say ">> no running systemd detected; skipping service enablement"
fi

say ""
say "Lisa Layer installed. Try:  lisa ask \"write a haiku about entropy\""
say "Uninstall cleanly with:     os/layer/uninstall.sh"
