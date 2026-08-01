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
    # A timer-driven client: it calls GNOME Online Accounts and writes
    # the user's maildir, and never identifies a peer — nothing
    # connects TO it. Its sandbox is real hardening at zero identity
    # cost. (This unit is also why the check reads install
    # destinations: its WantedBy lives in the .timer, and the first
    # version of the check never saw it.)
    "lisa-mail-sync.service",
}

def user_unit_sources(root: Path) -> set:
    """Unit files installed under systemd/user, by reading the installers.

    The first version of this check classified a unit as per-user by
    grepping its own text for WantedBy=default.target — which misses
    every timer-activated and D-Bus-activated user unit, because their
    activation lives in the .timer file or in BusName=, not in
    [Install]. lisa-mail-sync.service shipped three class options
    through exactly that gap while the check printed nothing. The
    ground truth is not what a unit says about itself but where the
    build installs it, so that is what is read: every PKGBUILD install
    line whose destination contains systemd/user (joining backslash
    continuations first — install lines wrap), plus anything sitting in
    a mkosi.extra systemd/user tree.
    """
    sources = set()
    for pkgbuild in (root / "os" / "packages").rglob("PKGBUILD"):
        text = pkgbuild.read_text().replace("\\\n", " ")
        for line in text.splitlines():
            if "systemd/user/" not in line:
                continue
            for tok in line.split():
                tok = tok.strip("\"'")
                if not tok.endswith(".service") or tok.startswith("$"):
                    continue
                # Resolve to a real repo file, or skip: destination
                # paths and enable-symlink targets are basenames of the
                # INSTALLED unit, which routinely differs from the repo
                # filename (lisa-remoted-user.service installs as
                # lisa-remoted.service), so a basename match would flag
                # the wrong file or none at all.
                src = root / tok
                if src.is_file():
                    sources.add(src.resolve())
    for extra in (root / "os").rglob("mkosi.extra/usr/lib/systemd/user/*.service"):
        sources.add(extra.resolve())
    return sources


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    user_units = user_unit_sources(root)
    if not user_units:
        # A matched-nothing sweep must fail, not pass: an installer
        # refactor that moves the install lines would otherwise turn
        # this check into a green no-op — the defect class it polices.
        print("check-user-units: found no per-user units at all — the "
              "installer scan is broken, not the tree clean", file=sys.stderr)
        return 1
    bad = []
    for unit in sorted(user_units):
        if unit.name in ALLOWED:
            continue
        text = unit.read_text()
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
