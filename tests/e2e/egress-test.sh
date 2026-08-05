#!/usr/bin/env bash
# Runtime egress proof for every no-egress daemon (CLAUDE.md rule 5, #275).
#
# WHAT THIS PROVES, AND FOR WHICH DAEMONS — read this before trusting it.
#
# For EVERY unit that os/repo-tools/check-egress-units.py classifies
# no-egress, this harness extracts that unit's own [Service] sandbox
# **from the shipped file**, applies it with `systemd-run -p`, and
# actually tries to reach the internet from inside it. That is a runtime
# proof, not a string assertion, and it now covers lisa-inferenced (both
# units), lisa-contextd, lisa-agentd, lisa-harnessd and lisa-notes —
# where before it covered lisa-inferenced alone, under a sandbox this
# script re-typed by hand and described as "mirrored exactly". Two lists
# that can silently disagree is the defect, so there is now one list:
# the units themselves.
#
# Only lisa-inferenced additionally gets its DAEMON run under the
# sandbox and asked to serve loopback, and the reason is practical
# rather than principled: it is the only one of the six that answers
# over HTTP. contextd, agentd, harnessd and notes are D-Bus/unix-socket
# daemons needing a session bus and a populated $XDG_RUNTIME_DIR to
# reach a state worth probing, neither of which a system-scope
# `systemd-run` in CI has. Their sandbox is proven; their liveness under
# it is not. That gap wants a session-scope harness, not a comment here
# claiming more than the code does.
#
# The suite is bracketed by two positive controls, because a test that
# can only fail open is decoration:
#   A. unsandboxed egress must SUCCEED (else "blocked" means "offline"),
#   B. egress under lisa-remoted's OWN shipped sandbox must SUCCEED —
#      rule 5's positive half, executed. If B fails, every "blocked"
#      above it is unattributable.
# Per unit there is a third: `curl --version` must run under that exact
# sandbox before its egress attempt counts. Otherwise a unit whose
# sandbox simply refuses to start a process would read as maximum
# security.
#
# Note what this deliberately does NOT do: attribute the block to a
# named directive. Deleting IPAddressDeny from lisa-agentd.service
# leaves it blocked by RestrictAddressFamilies=AF_UNIX, and this suite
# stays green — correctly, because the property still holds. Noticing
# the lost layer is check-egress-units.py's job. Run both.
#
# Usage: egress-test.sh [path/to/lisa-inferenced]
# Linux with systemd + sudo. Without the binary argument the daemon
# liveness check is skipped and everything else still runs.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CLASSIFY="$ROOT/os/repo-tools/check-egress-units.py"
PROBE_URL="${LISA_EGRESS_PROBE_URL:-http://example.com}"
CURL=$(command -v curl)

[ "$(uname)" = "Linux" ] || {
    echo "usage: $0 [path/to/lisa-inferenced] (Linux with systemd + sudo)" >&2
    exit 1
}

BIN=""
if [ $# -ge 1 ] && [ -n "$1" ]; then
    BIN=$(realpath "$1")
    [ -x "$BIN" ] || { echo "not executable: $BIN" >&2; exit 1; }
fi

fail=0
note() { printf '\n== %s\n' "$*"; }
bad() { echo "FAIL: $*" >&2; fail=1; }

# Directives that describe WHAT the unit runs and WHEN, not what it is
# allowed to do. Everything else in [Service] is passed through, so a
# sandbox directive added to a unit tomorrow is exercised tomorrow —
# which is the point of deriving instead of listing. Deny-list, never
# allow-list: an allow-list is the second list again.
#
# The *Directory= family is dropped for a reason worth writing down:
# it is state plumbing, not confinement, and seven units all asking for
# `StateDirectory=lisa` on ONE host collide. lisa-inferenced's
# DynamicUser run leaves /var/lib/lisa as a symlink into
# /var/lib/private owned by a dynamic uid, and the next unit's
# non-DynamicUser StateDirectory then dies 238/STATE_DIRECTORY — a test
# artifact that arrives disguised as "the sandbox blocked it". The
# per-unit `curl --version` control is what caught it.
NOT_SANDBOX="ExecStart ExecStartPre ExecStartPost ExecStop ExecStopPost \
ExecReload ExecCondition Type Restart RestartSec BusName Sockets \
Environment EnvironmentFile TimeoutStartSec TimeoutStopSec TimeoutSec \
Nice IOSchedulingClass IOSchedulingPriority OOMScoreAdjust \
RemainAfterExit NotifyAccess WatchdogSec KillMode KillSignal Slice \
StateDirectory StateDirectoryMode RuntimeDirectory RuntimeDirectoryMode \
CacheDirectory LogsDirectory ConfigurationDirectory WorkingDirectory"

# Emit `-p` and `KEY=VALUE` for one unit file, one array element each.
props_for() {
    python3 - "$1" "$NOT_SANDBOX" <<'PY'
import sys
path, skip = sys.argv[1], set(sys.argv[2].split())

# Transient units get NO specifier expansion: %h/%t are expanded when
# systemd PARSES a unit file, and a property handed to systemd-run
# never goes through that. Unexpanded, `ReadWritePaths=%h/...` is
# rejected outright ("Invalid ReadWritePaths") and the unit never
# starts — again indistinguishable from a working sandbox.
#
# These are the SYSTEM-scope values even for units that ship per-user,
# because system scope is what CI's `systemd-run` has. It changes which
# directory a path directive points at; it changes nothing about
# IPAddressDeny, IPAddressAllow or RestrictAddressFamilies, which are
# the three this test exists to exercise.
SPECIFIERS = {
    "%h": "/root", "%t": "/run", "%S": "/var/lib", "%C": "/var/cache",
    "%L": "/var/log", "%E": "/etc", "%u": "root", "%U": "0",
}

section = None
for raw in open(path):
    line = raw.strip()
    if not line or line[0] in "#;":
        continue
    if line.startswith("[") and line.endswith("]"):
        section = line[1:-1]
        continue
    if section != "Service" or "=" not in line:
        continue
    key, value = (p.strip() for p in line.split("=", 1))
    if key in skip:
        continue
    for spec, expansion in SPECIFIERS.items():
        value = value.replace(spec, expansion)
    # A path sandbox naming a directory that does not exist on the test
    # host also fails the unit at START. The `-` prefix is systemd's own
    # "ignore if missing" and weakens nothing this test asserts.
    if key in ("ReadWritePaths", "ReadOnlyPaths", "InaccessiblePaths"):
        value = " ".join(
            v if v.startswith("-") else "-" + v for v in value.split()
        )
    print("-p")
    print(f"{key}={value}")
PY
}

run_sandboxed() {  # unit file, then command...
    local unit="$1"; shift
    local props=()
    mapfile -t props < <(props_for "$unit")
    sudo systemd-run --wait --pipe --quiet "${props[@]}" -- "$@"
}

note "control A: egress works with NO sandbox (else every result below is 'offline', not 'blocked')"
if sudo systemd-run --wait --pipe --quiet -- "$CURL" -sf -m 20 -o /dev/null "$PROBE_URL"; then
    echo "ok: reached $PROBE_URL unsandboxed"
else
    echo "FAIL: cannot reach $PROBE_URL at all — this host has no egress to block." >&2
    exit 1
fi

note "no-egress daemons: each shipped unit's OWN sandbox, applied and attacked"
mapfile -t NO_EGRESS_UNITS < <(python3 "$CLASSIFY" --list no-egress)
[ "${#NO_EGRESS_UNITS[@]}" -gt 0 ] || {
    echo "FAIL: the classifier returned no no-egress units — discovery is broken," \
         "and an empty loop passes." >&2
    exit 1
}
for rel in "${NO_EGRESS_UNITS[@]}"; do
    unit="$ROOT/$rel"
    echo "-- $rel"
    props_for "$unit" | paste - - | sed 's/^/     /'

    # Per-unit control: this exact sandbox can start this exact binary.
    if ! run_sandboxed "$unit" "$CURL" --version >/dev/null 2>&1; then
        bad "$rel: curl will not even START under this sandbox — the egress" \
            "result below would be vacuous. Fix the harness or the unit."
        continue
    fi
    # The proof.
    if run_sandboxed "$unit" "$CURL" -sf -m 15 -o /dev/null "$PROBE_URL" 2>/dev/null; then
        bad "$rel: reached $PROBE_URL from inside the SHIPPED sandbox — this" \
            "daemon has egress and CLAUDE.md rule 5 says it must not."
    else
        echo "     ok: $PROBE_URL unreachable under the shipped sandbox"
    fi
done

mapfile -t EXEMPT_UNITS < <(python3 "$CLASSIFY" --list exempt)
if [ "${#EXEMPT_UNITS[@]}" -gt 0 ]; then
    note "NOT tested: units classified no-egress whose shipped file has no egress sandbox yet"
    for rel in "${EXEMPT_UNITS[@]}"; do
        echo "-- $rel: exempt in check-egress-units.py. Running it here would"
        echo "   only re-report a debt that gate already fails on the day it is"
        echo "   paid. Verified by hand on 2026-08-05: a process under this"
        echo "   unit's sandbox DOES reach the internet."
    done
fi

note "control B: lisa-remoted's OWN shipped sandbox still reaches the network (rule 5's positive half)"
mapfile -t EGRESS_UNITS < <(python3 "$CLASSIFY" --list egress)
[ "${#EGRESS_UNITS[@]}" -gt 0 ] || {
    echo "FAIL: no egress-permitted unit found — 'only remoted' is unrepresented." >&2
    exit 1
}
for rel in "${EGRESS_UNITS[@]}"; do
    unit="$ROOT/$rel"
    if run_sandboxed "$unit" "$CURL" -sf -m 20 -o /dev/null "$PROBE_URL" 2>/dev/null; then
        echo "ok: $rel reaches $PROBE_URL — the broker can broker"
    else
        bad "$rel is the sole egress broker and cannot reach $PROBE_URL." \
            "Either its sandbox over-blocks, or this host is offline and every" \
            "PASS above is meaningless."
    fi
done

# The only daemon with an HTTP surface, so the only one whose liveness
# under its own sandbox can be asserted here. See the header.
if [ -n "$BIN" ]; then
    note "lisa-inferenced serves loopback while under its own shipped sandbox"
    UNIT="$ROOT/os/packages/lisa/lisa-inferenced.service"
    # ProtectHome/PrivateTmp hide /home and /tmp, so the binary must live
    # somewhere the service can still see.
    RUN_BIN=/usr/local/bin/lisa-egress-test-bin
    sudo install -m755 "$BIN" "$RUN_BIN"
    cleanup() {
        sudo systemctl stop lisa-egress-daemon.service 2>/dev/null || true
        sudo rm -f "$RUN_BIN"
    }
    trap cleanup EXIT

    mapfile -t dprops < <(props_for "$UNIT")
    sudo systemd-run --unit=lisa-egress-daemon "${dprops[@]}" -- \
        "$RUN_BIN" --engine llama --models-dir /var/lib/lisa-models/refs
    up=""
    for _ in $(seq 1 50); do
        "$CURL" -sf 127.0.0.1:7777/health >/dev/null 2>&1 && { up=1; break; }
        sleep 0.2
    done
    if [ -n "$up" ] && "$CURL" -sf 127.0.0.1:7777/health | grep -q '"status":"ok"'; then
        echo "ok: /health served under the shipped sandbox"
    else
        bad "lisa-inferenced never answered /health under its own unit sandbox" \
            "— loopback is supposed to survive the egress block."
        sudo journalctl -u lisa-egress-daemon --no-pager -n 40 || true
    fi
fi

if [ "$fail" -ne 0 ]; then
    printf '\nEGRESS: FAIL\n' >&2
    exit 1
fi
printf '\nEGRESS: PASS (%d no-egress sandboxes blocked, broker unblocked)\n' \
    "${#NO_EGRESS_UNITS[@]}"
