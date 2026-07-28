#!/usr/bin/env bash
# Make this machine's SSH identity outlive the image it is running
# (issue #142).
#
# /etc is part of the per-slot root, so an A/B update replaces it whole
# and takes the host keys with it. The first login after every update is
# then:
#
#     @    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @
#
# That is the one warning that must never become routine — once a user
# learns to click through it, a real man-in-the-middle is
# indistinguishable from a Tuesday. It also breaks everything keyed to
# the host: known_hosts, monitoring, fleet inventory, git over SSH.
#
# Identity belongs to the machine, not to the slot, so the keys live on
# the durable `var` partition and are copied into place before sshd
# starts.
#
# COPY, not a bind mount over /etc/ssh: that directory also holds
# sshd_config, ssh_config and moduli, and binding a key store over it
# would hide them and break sshd outright.
#
# Boot-safe throughout: every step tolerates failure and on any problem
# the slot's own keys stay in use. A machine whose host key changed is
# still reachable; one that failed to boot is not.
set -uo pipefail

STORE=/var/lib/lisa/identity/ssh

# /var must be the durable partition (its own mount). If it is not there
# is nothing to persist onto, because the root fs is per-slot too.
mountpoint -q /var || exit 0

mkdir -p "$STORE" || exit 0
chmod 0755 "$STORE" 2>/dev/null || true

# --- 1. Seed the store, once ------------------------------------------
if ! compgen -G "$STORE/ssh_host_*_key" >/dev/null; then
  if compgen -G "/etc/ssh/ssh_host_*_key" >/dev/null; then
    # TRACK L (pacman layer on an already-running Arch/Omarchy box).
    # This machine has been keyed by sshd for however long, and the user
    # has trusted that identity. Adopt it rather than minting a new one:
    # installing a package must not invalidate anyone's known_hosts.
    cp -a /etc/ssh/ssh_host_*_key "$STORE/" 2>/dev/null || true
    cp -a /etc/ssh/ssh_host_*_key.pub "$STORE/" 2>/dev/null || true
  else
    # TRACK I (immutable A/B image) reaches this branch, always. A slot's
    # /etc comes from the image, and the image ships no host keys — it
    # must not, or every Lisa machine would share one identity. So on the
    # boot that first runs this script the slot is unkeyed, the store is
    # empty, and there is nothing to adopt: the machine's previous
    # identity lives in the OTHER slot's /etc, which is not mounted.
    #
    # That costs exactly ONE host-key change per machine, on the update
    # that introduces this script, and none afterwards. Reaching into the
    # inactive slot to avoid it would mean mounting an unverified
    # filesystem in early boot to obtain a private key — a worse trade
    # than one warning, once.
    #
    # Generating HERE (rather than letting sshd do it and adopting next
    # boot) is what keeps it to one: sshd's keys would be per-slot, so
    # the following update would change them a second time.
    ssh-keygen -q -t ed25519 -N "" -f "$STORE/ssh_host_ed25519_key" 2>/dev/null || true
    ssh-keygen -q -t rsa -b 4096 -N "" -f "$STORE/ssh_host_rsa_key" 2>/dev/null || true
    ssh-keygen -q -t ecdsa -b 521 -N "" -f "$STORE/ssh_host_ecdsa_key" 2>/dev/null || true
  fi
fi

# Nothing to install (generation failed, or no ssh-keygen): leave the
# slot exactly as it was.
compgen -G "$STORE/ssh_host_*_key" >/dev/null || exit 0

# --- 2. Install into this slot ----------------------------------------
# Unconditional every boot: this is what makes a NEW slot adopt the
# machine's existing identity instead of minting its own.
# Modes are set explicitly rather than via `install -o root -g root`:
# that form fails outright when not run as root, and `|| true` would
# then hide a private key that never got installed. sshd REFUSES to
# start on a group/world-readable host key, so 0600 here is load-bearing
# — and a public key left at 0600 is the mirror-image bug, silently
# breaking anything that reads it.
mkdir -p /etc/ssh || exit 0
for key in "$STORE"/ssh_host_*_key; do
  [ -f "$key" ] || continue
  base=$(basename "$key")
  cp -f "$key" "/etc/ssh/$base" 2>/dev/null && chmod 0600 "/etc/ssh/$base" 2>/dev/null || true
  if [ -f "$key.pub" ]; then
    cp -f "$key.pub" "/etc/ssh/$base.pub" 2>/dev/null \
      && chmod 0644 "/etc/ssh/$base.pub" 2>/dev/null || true
  fi
done

exit 0
