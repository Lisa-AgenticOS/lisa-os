#!/usr/bin/env python3
"""The two A/B root slots must be the same fixed size.

systemd-sysupdate installs an update by writing slot A's byte image into
slot B. If B is smaller the write is truncated; if it is larger the
filesystem is short of the partition and only growfs saves it. Neither
failure announces itself at build time — the image builds, boots, and
breaks on the first update, which is the most expensive moment to find
out.

The sizes live in two files precisely so they can drift, and a comment
saying "MUST match" is not a mechanism. This is.

Run by `just lint`; costs milliseconds and no image build.
"""

import re
import sys
from pathlib import Path

REPART = Path(__file__).resolve().parents[2] / "os" / "mkosi" / "mkosi.repart"
ROOT_SLOTS = ("10-root.conf", "11-root-b.conf")


def sizes(path: Path) -> dict:
    """SizeMinBytes/SizeMaxBytes from one repart drop-in."""
    out = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        if key.strip() in ("SizeMinBytes", "SizeMaxBytes"):
            out[key.strip()] = value.strip()
    return out


def main() -> int:
    fail = False
    seen = {}
    for name in ROOT_SLOTS:
        path = REPART / name
        if not path.is_file():
            print(f"FAIL: {path} is missing — the A/B layout needs both slots")
            return 1
        s = sizes(path)
        # A slot without both bounds is not a fixed-size slot, and the
        # byte-image update assumes fixed size.
        for key in ("SizeMinBytes", "SizeMaxBytes"):
            if key not in s:
                print(f"FAIL: {name} has no {key} — root slots must be fixed size")
                fail = True
        if s.get("SizeMinBytes") != s.get("SizeMaxBytes"):
            print(
                f"FAIL: {name} is not fixed size "
                f"({s.get('SizeMinBytes')} != {s.get('SizeMaxBytes')})"
            )
            fail = True
        seen[name] = s.get("SizeMinBytes")

    a, b = (seen.get(n) for n in ROOT_SLOTS)
    if a != b:
        print(f"FAIL: root slot sizes differ — {ROOT_SLOTS[0]}={a}, {ROOT_SLOTS[1]}={b}")
        print("      sysupdate writes A's byte image into B; a mismatch corrupts installs.")
        fail = True

    # A sweep that matched nothing must not read as success — the lesson
    # from check-user-units.py, which classified units by a field that
    # stopped existing and stayed silently green.
    if not any(seen.values()):
        print("FAIL: parsed no sizes at all — this check is not checking anything")
        return 1

    if fail:
        return 1
    print(f"repart root slots: ok (A/B both {a})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
