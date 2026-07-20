#!/usr/bin/env bash
# Egress verification harness (PLAN §5.10: "verifiable > promised").
# Runs the daemon under the same sandbox directives as
# os/packages/lisa/lisa-inferenced.service on a real systemd host (CI
# ubuntu runner or any Linux with sudo) and proves:
#   1. the loopback endpoint works under the sandbox,
#   2. the sandbox blocks outbound network entirely,
#   3. the block is the sandbox's doing (control probe succeeds without it).
# M1 upgrade path: nftables cgroup packet counter over the full test
# suite, per the §5.1 acceptance block.
set -euo pipefail

BIN=$(realpath "${1:-target/debug/lisa-inferenced}")
[ -x "$BIN" ] && [ "$(uname)" = "Linux" ] || {
    echo "usage: $0 [path/to/lisa-inferenced] (Linux with systemd + sudo)" >&2
    exit 1
}
PROBE_URL="http://example.com"

# The sandbox hides /home and /tmp (ProtectHome, PrivateTmp), so the
# binary must live somewhere the service can still see.
RUN_BIN=/usr/local/bin/lisa-egress-test-bin
sudo install -m755 "$BIN" "$RUN_BIN"

# The unit's network-relevant sandbox, mirrored exactly.
SANDBOX=(
    -p DynamicUser=yes
    -p IPAddressDeny=any
    -p IPAddressAllow=localhost
    -p RestrictAddressFamilies="AF_UNIX AF_INET AF_INET6"
    -p ProtectSystem=strict
    -p ProtectHome=yes
    -p NoNewPrivileges=yes
    -p PrivateTmp=yes
)

cleanup() {
    sudo systemctl stop lisa-egress-daemon.service 2>/dev/null || true
    sudo rm -f "$RUN_BIN"
}
trap cleanup EXIT

echo "== control: egress works OUTSIDE the sandbox (else this test is vacuous)"
sudo systemd-run --wait --pipe -p DynamicUser=yes curl -sf -m 15 "$PROBE_URL" >/dev/null

echo "== start daemon under the unit's sandbox"
sudo systemd-run --unit=lisa-egress-daemon "${SANDBOX[@]}" "$RUN_BIN"
for _ in $(seq 1 50); do
    curl -sf 127.0.0.1:7777/health >/dev/null 2>&1 && break
    sleep 0.2
done

echo "== loopback serving works under the sandbox"
curl -sf 127.0.0.1:7777/health | grep -q '"status":"ok"'

echo "== egress from inside the same sandbox must fail"
if sudo systemd-run --wait --pipe "${SANDBOX[@]}" curl -sf -m 10 "$PROBE_URL" >/dev/null 2>&1; then
    echo "FAIL: sandboxed process reached $PROBE_URL — egress is NOT blocked" >&2
    exit 1
fi

echo "== loopback still healthy after the egress attempt"
curl -sf 127.0.0.1:7777/health >/dev/null

echo "EGRESS: PASS (loopback served, outbound blocked)"
