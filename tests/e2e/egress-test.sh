#!/usr/bin/env bash
# Runtime egress proof for every no-egress daemon (CLAUDE.md rule 5, #275).
#
# WHAT THIS PROVES, AND FOR WHICH DAEMONS — read this before trusting it.
#
# For EVERY unit that os/repo-tools/check-egress-units.py classifies
# no-egress, this harness extracts that unit's own [Service] sandbox
# **from the shipped file, plus its shipped drop-ins**, applies it with
# `systemd-run -p`, and actually tries to reach the internet from inside
# it. That is a runtime proof, not a string assertion. The population is
# whatever `--list no-egress` prints — deliberately NOT enumerated here,
# because the enumeration that used to sit in this comment had already
# drifted (it named six units while the gate classified seven, #295),
# which is the exact defect the derived list was introduced to remove.
# Run `check-egress-units.py --explain` to see it.
#
# The drop-ins are #292 and they are the reason this comment changed. A
# `<unit>.service.d/*.conf` handing the network back was invisible to
# BOTH layers at once: the static gate never opened a `.conf`, and this
# script rebuilt the sandbox from the pristine unit file, so it applied
# a confinement the machine does not have and reported "blocked". The
# harness now asks the classifier for the drop-ins (`--dropins`) and
# applies them in systemd's order, after the unit.
#
# Only lisa-inferenced additionally gets its DAEMON run under the
# sandbox and asked to serve loopback, and the reason is practical
# rather than principled: it is the only no-egress daemon that answers
# over HTTP. The rest of the population — ask `--list no-egress`, it is
# deliberately not enumerated here, because the two enumerations this
# file used to carry both drifted (#295, twice) — are D-Bus/unix-socket
# daemons and portals needing a session bus and a populated
# $XDG_RUNTIME_DIR to reach a state worth probing, neither of which a
# system-scope `systemd-run` in CI has. Their sandbox is proven; their
# liveness under it is not. That gap wants a session-scope harness, not
# a comment here
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

# Emit `-p` and `KEY=VALUE` for a unit file AND its drop-ins, one array
# element each. Several files, in the order systemd applies them, so a
# drop-in's `IPAddressDeny=` reset lands after the unit's
# `IPAddressDeny=any` exactly as it would on the machine (#292).
props_for() {
    python3 - "$NOT_SANDBOX" "$@" <<'PY'
import sys
skip, paths = set(sys.argv[1].split()), sys.argv[2:]

# Transient units get NO specifier expansion: %h/%t are expanded when
# systemd PARSES a unit file, and a property handed to systemd-run
# never goes through that. Unexpanded, `ReadWritePaths=%h/...` is
# rejected outright ("Invalid ReadWritePaths") and the unit never
# starts — again indistinguishable from a working sandbox.
#
# These are the SYSTEM-scope values even for units that ship per-user,
# because system scope is what CI's `systemd-run` has. It changes which
# directory a path directive points at.
#
# It ALSO changes IPAddressDeny and IPAddressAllow, and this comment
# used to claim the opposite (#295). Those two are a cgroup BPF
# program: root can load one, `systemd --user` cannot, and the user
# manager says so in the journal —
#
#   lisa-agentd.service: unit configures an IP firewall, but not
#   running as root.
#
# — so a per-user unit's IP filter is INERT on the machine and LIVE
# here. That makes this harness stricter than reality for those two
# directives, never weaker, but a "blocked" it produces for a user unit
# may be a block the device does not have. RestrictAddressFamilies is
# a seccomp filter and behaves identically in both scopes; it is the
# only one of the three that confines a user unit, which is #288 and
# why check-egress-units.py asserts it separately.
SPECIFIERS = {
    "%h": "/root", "%t": "/run", "%S": "/var/lib", "%C": "/var/cache",
    "%L": "/var/log", "%E": "/etc", "%u": "root", "%U": "0",
}

section = None
for path in paths:
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
        # A path sandbox naming a directory that does not exist on the
        # test host also fails the unit at START. The `-` prefix is
        # systemd's own "ignore if missing" and weakens nothing this
        # test asserts.
        if key in ("ReadWritePaths", "ReadOnlyPaths", "InaccessiblePaths"):
            value = " ".join(
                v if v.startswith("-") else "-" + v for v in value.split()
            )
        print("-p")
        print(f"{key}={value}")
PY
}

# The unit file and every drop-in that ships with it, absolute paths, in
# application order. Derived, never listed — the same reason the unit
# population is (#292, #295).
files_for() {  # repo-relative unit path
    local rel="$1" d
    printf '%s\n' "$ROOT/$rel"
    while IFS= read -r d; do
        [ -n "$d" ] && printf '%s\n' "$ROOT/$d"
    done < <(python3 "$CLASSIFY" --dropins "$rel")
}

run_sandboxed() {  # repo-relative unit path, then command...
    local rel="$1"; shift
    local files=() props=()
    mapfile -t files < <(files_for "$rel")
    mapfile -t props < <(props_for "${files[@]}")
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
    echo "-- $rel"
    mapfile -t FILES < <(files_for "$rel")
    # Printed, not assumed: a drop-in silently missing from this list is
    # the failure #292 describes, and the only way to see it is to say
    # which files the sandbox below was built from.
    for f in "${FILES[@]}"; do echo "     from ${f#"$ROOT/"}"; done
    props_for "${FILES[@]}" | paste - - | sed 's/^/     /'

    # Per-unit control: this exact sandbox can start this exact binary.
    if ! run_sandboxed "$rel" "$CURL" --version >/dev/null 2>&1; then
        bad "$rel: curl will not even START under this sandbox — the egress" \
            "result below would be vacuous. Fix the harness or the unit."
        continue
    fi
    # The proof.
    if run_sandboxed "$rel" "$CURL" -sf -m 15 -o /dev/null "$PROBE_URL" 2>/dev/null; then
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
        echo "-- $rel: exempt in check-egress-units.py — its shipped file has"
        echo "   no egress sandbox, so there is no fence to test. The gate"
        echo "   carries the debt and fails the day the unit is hardened;"
        echo "   until then assume a process under it reaches the internet."
    done
fi

note "control B: lisa-remoted's OWN shipped sandbox still reaches the network (rule 5's positive half)"
mapfile -t EGRESS_UNITS < <(python3 "$CLASSIFY" --list egress)
[ "${#EGRESS_UNITS[@]}" -gt 0 ] || {
    echo "FAIL: no egress-permitted unit found — 'only remoted' is unrepresented." >&2
    exit 1
}
for rel in "${EGRESS_UNITS[@]}"; do
    if run_sandboxed "$rel" "$CURL" -sf -m 20 -o /dev/null "$PROBE_URL" 2>/dev/null; then
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
    UNIT_REL=os/packages/lisa/lisa-inferenced.service
    # ProtectHome/PrivateTmp hide /home and /tmp, so the binary must live
    # somewhere the service can still see.
    RUN_BIN=/usr/local/bin/lisa-egress-test-bin
    sudo install -m755 "$BIN" "$RUN_BIN"
    cleanup() {
        sudo systemctl stop lisa-egress-daemon.service 2>/dev/null || true
        sudo rm -f "$RUN_BIN"
    }
    trap cleanup EXIT

    mapfile -t dfiles < <(files_for "$UNIT_REL")
    mapfile -t dprops < <(props_for "${dfiles[@]}")
    # Give back the ONE thing NOT_SANDBOX took away, and nothing else.
    #
    # props_for drops StateDirectory= on purpose (see NOT_SANDBOX): it is
    # state plumbing rather than confinement, and seven units all asking
    # for `StateDirectory=lisa` on one host collide. But dropping it
    # while KEEPING ProtectSystem=strict leaves the daemon a read-only
    # /var, so lisa-inferenced died on
    #
    #   cannot open ledger /var/lib/lisa/ledger.db: Read-only file system
    #
    # before it ever bound a port — and the check below reported "never
    # answered /health", which reads as "the sandbox broke loopback". It
    # did not. The harness broke the daemon. That is precisely the
    # failure mode this file exists to prevent: a unit that never
    # started, indistinguishable from a sandbox that worked.
    #
    # Restoring it is safe HERE and nowhere else in this script. The
    # collision NOT_SANDBOX guards against is between the several units
    # the loop above probes; this is one unit, run alone, after that loop
    # has finished. So the daemon gets the directive its unit actually
    # ships, verbatim.
    #
    # Verbatim matters. The first attempt granted ReadWritePaths= on a
    # root-owned /var/lib/lisa instead, and the daemon still failed —
    # `unable to open database file` rather than `Read-only file system`,
    # because the unit is DynamicUser=yes and a dynamic uid cannot write
    # a directory root made. StateDirectory= is what creates that
    # directory OWNED BY the dynamic user, at StateDirectoryMode=0700.
    # Reimplementing a systemd directive by hand got its permissions
    # wrong on the first try, which is the argument for not
    # reimplementing it.
    dprops+=(-p StateDirectory=lisa -p StateDirectoryMode=0700)
    sudo systemd-run --unit=lisa-egress-daemon "${dprops[@]}" -- \
        "$RUN_BIN" --engine llama --models-dir /var/lib/lisa-models/refs
    up=""
    for _ in $(seq 1 50); do
        "$CURL" -sf 127.0.0.1:7777/health >/dev/null 2>&1 && { up=1; break; }
        sleep 0.2
    done
    if [ -n "$up" ] && "$CURL" -sf 127.0.0.1:7777/health | grep -q '"status":"ok"'; then
        echo "ok: /health served under the shipped sandbox"
        # The StateDirectory= above changes the property set the daemon
        # runs under, so the egress block must be re-proven for THAT set
        # — not inferred from the curl probe a few lines up, which ran
        # without it. Same properties, network instead of loopback: it
        # must still fail.
        if sudo systemd-run --wait --pipe --quiet "${dprops[@]}" -- \
               "$CURL" -sf -m 20 -o /dev/null "$PROBE_URL" 2>/dev/null; then
            bad "the daemon's property set — the one with StateDirectory=" \
                "restored — reaches $PROBE_URL. Writable state was supposed" \
                "to restore the ledger, not the network."
        else
            echo "ok: the same property set still cannot reach $PROBE_URL"
        fi
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
