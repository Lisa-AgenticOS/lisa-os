#!/usr/bin/env python3
"""Every shipped unit that runs a Lisa daemon declares an egress posture (#275).

CLAUDE.md rule 5: `lisa-inferenced`, `lisa-contextd` and `lisa-agentd`
never get network access; **only `lisa-remoted`** does — it is the sole
egress broker. That is architecture, so it has to be a mechanism.

What went wrong on 2026-08-05, and why this file exists:

  * `os/packages/lisa/lisa-inferenced-dbus.service` — the per-user
    companion every Assistant and overlay prompt goes through — shipped
    with **no egress sandbox at all**, while two comments inside it
    described one. Rule 5 was prose, on the daemon that handles every
    prompt.
  * The one thing CI did check was a hand-typed loop over six unit
    paths. A seventh unit, or a rename, is invisible to a list nobody
    updates. The `egress` job's own comment admitted this.

So the population is *discovered*, never typed:

  1. Read the installers (`PKGBUILD` install lines into `systemd/system/`
     or `systemd/user/`, plus mkosi's baked-in unit trees). Where the
     build puts a unit is the ground truth about what ships — the same
     reasoning check-user-units.py arrived at, for the same reason: a
     unit's own `[Install]` section lies by omission for timer- and
     D-Bus-activated services.
  2. For each unit take the `ExecStart=` binary. If it is a Lisa binary
     (`lisa`, `lisa-*`, `xdg-desktop-portal-lisa`) it MUST have a
     posture below. **An unknown Lisa binary fails this check** — that
     is the whole design. A fourth no-egress daemon added next year
     cannot slip in unclassified, and a second unit for an already-known
     daemon (which is exactly what lisa-inferenced-dbus was) is picked
     up with no edit here at all.
  3. Units that run something else — mkosi's boot shell scripts, stock
     binaries — are out of rule 5's scope and are skipped by name in
     `--explain` output rather than silently.

The posture is keyed by BINARY, not by unit filename, because the
property is a property of the daemon: lisa-contextd may not reach the
network in a system unit, a user unit, or any unit written after this
comment.

Static assertion only. The runtime proof — actually attempting egress
from inside each shipped sandbox and watching it fail — is
`tests/e2e/egress-test.sh`, which takes its unit list from this file
(`--list no-egress`) so the two can never be two lists. This check is
still worth having on its own: the runtime test proves *the sandbox as a
whole* blocks egress, which stays true if `IPAddressDeny` is deleted
from a unit that also has `RestrictAddressFamilies=AF_UNIX`. Defence in
depth only counts if losing a layer is noticed, and only a static check
can notice that.

Usage:
    check-egress-units.py             # the gate
    check-egress-units.py --explain   # print the classification table
    check-egress-units.py --list no-egress|egress|exempt|cli
"""

import sys
from pathlib import Path

# ---------------------------------------------------------------- postures

# Binaries that must never reach the network. Value = why, in the terms
# the unit itself uses; if a daemon's answer to "what does it talk to"
# ever includes an address off this machine, it belongs in EGRESS and
# needs an ADR, not an edit here.
NO_EGRESS = {
    "lisa-inferenced": (
        "PLAN §5.1 model runtime. Serves loopback (7777 system, 7778 user) "
        "and a unix socket; the BYO remote tier is reached by handing the "
        "request to lisa-remoted over %t/lisa/remoted.sock, so the engine "
        "itself never opens a route off the machine."
    ),
    "lisa-contextd": (
        "PLAN §5.3 context fabric. D-Bus surface, local SQLite store. It "
        "reads the user's consented index — the daemon with the most to "
        "leak and the least reason to."
    ),
    "lisa-agentd": (
        "PLAN §5.4 Agent Bus. D-Bus plus per-app MCP unix sockets. Tool "
        "dispatch that could reach the network would make every tier and "
        "provenance decision advisory."
    ),
    "lisa-harnessd": (
        "ADR-0025 agent loop. Hosts the MODEL, and is allowed loopback "
        "(IPAddressAllow=localhost) to reach its inferenced companion on "
        ":7778 — no further. A model endpoint that could be pointed at "
        "the internet by configuration is an egress channel with a "
        "config file for a guard."
    ),
    "lisa-notes": (
        "ADR-0013 first-party MCP server. Unix socket plus SQLite under "
        "$XDG_DATA_HOME. Every app tool provider inherits this posture."
    ),
    "xdg-desktop-portal-lisa": (
        "PLAN §5.5 trust boundary. It brokers fds and consent; it never "
        "carries model traffic, as its own unit says."
    ),
}

# Binaries permitted egress. Rule 5's positive half: this set should
# have exactly one entry that ships a unit, and adding to it is an ADR.
EGRESS = {
    "lisa-remoted": (
        "ADR-0010 / PLAN §5.11 — THE egress broker, and the only one. "
        "Every request it makes is ledgered with the `remote.` "
        "'leaves your hardware' marking behind default-deny per-scope "
        "consent."
    ),
    "lisa-modeld": (
        "PLAN §5.2 content-addressed model store. Fetches model "
        "artifacts. Ships no unit today (#275 item 3 stays open), so "
        "nothing below will match it — the entry is here so that the "
        "day it gets one, this file does not fail for the wrong reason."
    ),
}

# The `lisa` CLI is not a daemon and its units are one-shots. Recorded
# rather than policed, and the awkward part is recorded too.
NOT_A_DAEMON = {
    "lisa": (
        "The user-facing CLI (`lisa apps sync`, `lisa mail sync`, "
        "`lisa context sync-knowledge`), invoked as Type=oneshot. Rule 5 "
        "governs daemons; a CLI's egress is governed by the guard "
        "catalogue and by remoted. Stated plainly because it is the "
        "loose end: lisa-mail-sync.service does reach an IMAP host "
        "directly rather than through the broker. That is a real "
        "divergence from 'only remoted', it predates this check, and it "
        "is not something a unit-file assertion can fix."
    ),
}

# No-egress binaries whose SHIPPED unit does not carry IPAddressDeny
# today. Each entry is a debt, not a dispensation — and the check FAILS
# if an exempt unit gains the directive, so the exemption deletes itself
# the moment the debt is paid. (Requiring the directive here instead
# would mean shipping a red gate, which teaches everyone to ignore it.)
# Empty, and that is the point. xdg-desktop-portal-lisa.service was the
# one entry: it carried the per-user hardening subset but no
# IPAddressDeny/RestrictAddressFamilies, on the component that IS the
# trust boundary (#285). The exemption lived for exactly as long as it
# took to harden the unit — the gate refused to pass with both the
# directive and the exemption present, which is the self-retiring
# mechanism working rather than a comment claiming it would.
EXEMPT = {}

# Rule 5 names three daemons. If discovery stops finding a unit for one
# of them — deleted, renamed, moved out of the installers — the check
# above would go quietly green over an empty set. This is the floor.
REQUIRED_NO_EGRESS_COVERAGE = ("lisa-inferenced", "lisa-contextd", "lisa-agentd")

# ...and the positive half needs a shipped unit too, or "only remoted"
# is an absence rather than a statement.
REQUIRED_EGRESS_COVERAGE = ("lisa-remoted",)

DIRECTIVE = "IPAddressDeny=any"


def shipped_units(root: Path) -> set:
    """Units this repo installs, by reading the installers.

    Backslash continuations are joined first (install lines wrap), and
    every whitespace token that resolves to a real repo file is taken as
    a source. Destination paths and enable-symlink targets are basenames
    of the INSTALLED unit, which routinely differs from the repo
    filename (lisa-remoted-user.service installs as
    lisa-remoted.service), so they resolve to nothing and drop out.
    """
    units = set()
    for pkgbuild in (root / "os" / "packages").rglob("PKGBUILD"):
        text = pkgbuild.read_text().replace("\\\n", " ")
        for line in text.splitlines():
            if "systemd/system/" not in line and "systemd/user/" not in line:
                continue
            for tok in line.split():
                tok = tok.strip("\"'")
                if not tok.endswith(".service") or tok.startswith("$"):
                    continue
                src = root / tok
                if src.is_file():
                    units.add(src.resolve())
    for tree in ("mkosi.extra", "initrd-overlay"):
        for unit in (root / "os").rglob(f"{tree}/usr/lib/systemd/*/*.service"):
            units.add(unit.resolve())
    return units


def service_lines(text: str):
    """([Service] directive, value, line number) for uncommented lines."""
    section = None
    for line_no, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if line.startswith("#") or line.startswith(";") or not line:
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1]
            continue
        if section != "Service" or "=" not in line:
            continue
        key, value = line.split("=", 1)
        yield key.strip(), value.strip(), line_no


def exec_binary(text: str):
    """Basename of the unit's ExecStart binary, or None.

    systemd allows `-`, `+`, `!`, `!!`, `:` and `@` prefixes on the
    command; strip them or a `-/usr/bin/lisa` reads as a binary called
    `-lisa` and lands in the unclassified pile for no reason.
    """
    for key, value, _ in service_lines(text):
        if key != "ExecStart" or not value:
            continue
        cmd = value.split()[0].lstrip("-+!:@")
        return Path(cmd).name
    return None


def is_lisa_binary(name: str) -> bool:
    return name == "lisa" or name.startswith("lisa-") or name == "xdg-desktop-portal-lisa"


def classify(root: Path):
    """[(unit path, binary, posture, has_directive)] for Lisa-binary units."""
    rows = []
    for unit in sorted(shipped_units(root)):
        text = unit.read_text()
        binary = exec_binary(text)
        if binary is None or not is_lisa_binary(binary):
            rows.append((unit, binary, "out-of-scope", False))
            continue
        if binary in NO_EGRESS:
            posture = "no-egress"
        elif binary in EGRESS:
            posture = "egress"
        elif binary in NOT_A_DAEMON:
            posture = "cli"
        else:
            posture = "UNCLASSIFIED"
        has = any(
            key == "IPAddressDeny" and value == "any"
            for key, value, _ in service_lines(text)
        )
        rows.append((unit, binary, posture, has))
    return rows


def main(argv) -> int:
    root = Path(__file__).resolve().parents[2]
    rows = classify(root)

    if "--list" in argv:
        want = argv[argv.index("--list") + 1]
        for unit, _binary, posture, _has in rows:
            # `no-egress` is what the runtime harness attacks, so an
            # exempt unit must not appear in it: it demonstrably HAS
            # egress today (tests/e2e/egress-test.sh watched the portal
            # reach the internet), and a harness that failed on a debt
            # this file already records would just be reporting the
            # same thing twice, red.
            exempt = unit.name in EXEMPT
            if want == "exempt" and posture == "no-egress" and exempt:
                print(unit.relative_to(root))
            elif want == posture and not (posture == "no-egress" and exempt):
                print(unit.relative_to(root))
        return 0

    if "--explain" in argv:
        for unit, binary, posture, has in rows:
            mark = "IPAddressDeny=any" if has else "-"
            print(f"{posture:14} {binary or '(no ExecStart)':26} {mark:18} "
                  f"{unit.relative_to(root)}")
        return 0

    errors = []

    if not any(posture != "out-of-scope" for _u, _b, posture, _h in rows):
        # A matched-nothing sweep must fail, not pass: an installer
        # refactor that moves the install lines would otherwise turn
        # this into a green no-op — the defect class it polices.
        errors.append(
            "discovery found no unit running a Lisa binary at all — the "
            "installer scan is broken, not the tree clean"
        )

    covered_no_egress = set()
    covered_egress = set()
    for unit, binary, posture, has in rows:
        rel = unit.relative_to(root)
        if posture == "UNCLASSIFIED":
            errors.append(
                f"{rel}: runs `{binary}`, which has no egress posture. Add it "
                f"to NO_EGRESS or EGRESS in {Path(__file__).name} with the "
                f"reason — a daemon nobody classified is a daemon nobody "
                f"checked (CLAUDE.md rule 5)."
            )
        elif posture == "no-egress":
            covered_no_egress.add(binary)
            if unit.name in EXEMPT:
                if has:
                    errors.append(
                        f"{rel}: now carries {DIRECTIVE} and is still listed in "
                        f"EXEMPT. Delete the exemption — it exists to be "
                        f"deleted."
                    )
            elif not has:
                errors.append(
                    f"{rel}: runs `{binary}`, which may never reach the "
                    f"network, and does not carry {DIRECTIVE}. The unit's "
                    f"comments are not a sandbox — lisa-inferenced-dbus.service "
                    f"shipped that way (#275)."
                )
        elif posture == "egress":
            covered_egress.add(binary)
            if has:
                errors.append(
                    f"{rel}: runs `{binary}`, the egress broker, but carries "
                    f"{DIRECTIVE}. Either the broker cannot broker, or its "
                    f"posture in {Path(__file__).name} is wrong."
                )

    for binary in REQUIRED_NO_EGRESS_COVERAGE:
        if binary not in covered_no_egress:
            errors.append(
                f"CLAUDE.md rule 5 names `{binary}` as a no-egress daemon, but "
                f"discovery found no shipped unit running it. Either it stopped "
                f"shipping, or the installer scan stopped seeing it; both mean "
                f"this check is no longer covering it."
            )
    for binary in REQUIRED_EGRESS_COVERAGE:
        if binary not in covered_egress:
            errors.append(
                f"`{binary}` is the sole permitted egress path, and no shipped "
                f"unit runs it. 'Only remoted' has to be a statement about "
                f"something that exists."
            )

    if errors:
        print("egress posture check FAILED:\n", file=sys.stderr)
        for e in errors:
            print(f"  {e}\n", file=sys.stderr)
        return 1

    n = sum(1 for _u, _b, p, _h in rows if p == "no-egress")
    m = sum(1 for _u, _b, p, _h in rows if p == "egress")
    print(f"egress posture: {n} no-egress units, {m} egress units, "
          f"{len(EXEMPT)} exemption(s) — OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
