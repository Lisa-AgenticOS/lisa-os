#!/usr/bin/env python3
"""Per-user units must not use sandbox options that imply PrivateUsers (#161).

A per-user systemd manager has no privileges, so every mount-based
sandbox option — ProtectSystem, ProtectHome, PrivateTmp,
ProtectKernelTunables, and the rest of the class below — can only be
applied by first putting the service in a private user namespace.
Inside that namespace, ptrace-read of any process outside it is
denied, so `/proc/<pid>/exe` (and `/proc/<pid>/root/.flatpak-info`)
of every peer returns EACCES.

That kills `lisa_peer::exe_of_peer`, and with it every ADR-0033
identity decision: the manager allowlist matches nothing, the portal
cannot tell one caller from another, and the user's own CLI and
Settings are refused by their own machine. The failure is silent and
fail-closed, which is the worst kind to ship: everything looks like a
security decision instead of a broken build.

Bisected on the reference iMac (systemd 261): ProtectKernelTunables,
ProtectKernelModules, ProtectHome, ProtectSystem=strict and
PrivateDevices each individually reproduce the EACCES; the uid_map of
such a service reads `1000 1000 1` — a one-uid namespace.

System units are unaffected: a root manager applies these options with
real mounts, no user namespace involved. So the rule is exactly:
**per-user units of daemons that identify peers get no options from
this class.** Units that never do kernel-side identification may keep
them, but each needs a justification in ALLOWED below.
"""

import re
import sys
from pathlib import Path

# The mount/namespace sandbox class — everything a user manager can
# only deliver via PrivateUsers. (systemd.exec: "these options are
# only available to the system service manager, unless the service is
# running in a user namespace".)
CLASS = (
    "ProtectSystem",
    "ProtectHome",
    "PrivateTmp",
    "PrivateDevices",
    "PrivateNetwork",
    "PrivateUsers",
    "PrivateIPC",
    "PrivateMounts",
    "ProtectKernelTunables",
    "ProtectKernelModules",
    "ProtectKernelLogs",
    "ProtectControlGroups",
    "ProtectClock",
    "ProtectHostname",
    "ProtectProc",
    "ProcSubset",
    "ReadOnlyPaths",
    "InaccessiblePaths",
    "ReadWritePaths",
    "TemporaryFileSystem",
    "BindPaths",
    "BindReadOnlyPaths",
    "MountAPIVFS",
)

# Per-user units allowed to keep class options, because the daemon
# behind them performs no kernel-side peer identification. Adding a
# unit here is a claim about its code: check for lisa_peer::exe_of_peer
# (and /proc/<pid>/ reads about peers) first.
ALLOWED = {
    # Ownership checks in harnessd are broker-name based (Owner), never
    # /proc-based; its confinement is load-bearing for ADR-0029.
    "lisa-harnessd.service",
}

USER_UNIT_MARKERS = re.compile(
    r"^WantedBy=(default\.target|graphical-session\.target)", re.M
)


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    bad = []
    for unit in sorted((root / "os" / "packages").rglob("*.service")):
        text = unit.read_text()
        if not USER_UNIT_MARKERS.search(text):
            continue  # system unit, or not installed into a session
        if unit.name in ALLOWED:
            continue
        for line_no, line in enumerate(text.splitlines(), 1):
            opt = line.split("=", 1)[0].strip()
            if opt in CLASS and not line.lstrip().startswith("#"):
                bad.append(f"{unit.relative_to(root)}:{line_no}: {line.strip()}")
    if bad:
        print(
            "per-user units using sandbox options that imply PrivateUsers —\n"
            "these break /proc/<pid>/exe for every peer and with it all\n"
            "ADR-0033 identity checks (#161):\n",
            file=sys.stderr,
        )
        for b in bad:
            print(f"  {b}", file=sys.stderr)
        print(
            "\nEither the daemon does no peer identification (add the unit\n"
            "to ALLOWED with a justification), or the option must go.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
