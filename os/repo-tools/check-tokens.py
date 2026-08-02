#!/usr/bin/env python3
"""Every color a Lisa surface hardcodes must be a token (ADR-0038 step 1).

The three-violets defect, mechanized: the desktop review of 2026-08-02
found #4F378B, #6D45C9 and #7A55D1 all standing in for "the brand
violet" because nothing failed when a surface invented its own. This
gate fails the build for any hex literal in shell/ or apps/ that is
not in branding/tokens.json — and runs the generator's --check so the
committed outputs cannot drift from the source either.

What it deliberately does NOT police: the websites, os/mkosi wallpaper
SVGs and Plymouth theme (asset files, not UI code — they get the brief,
not the linter), and test fixtures (a test may name any color it wants
to assert about).
"""

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

HEX = re.compile(r"#[0-9a-fA-F]{6}\b")
SURFACES = ["shell", "apps"]
SUFFIXES = {".js", ".css"}


def main():
    tokens = json.loads((ROOT / "branding/tokens.json").read_text())
    sanctioned = {
        spec["value"].lower()
        for entries in tokens["color"].values()
        for spec in entries.values()
    }

    bad = []
    for surface in SURFACES:
        for path in sorted((ROOT / surface).rglob("*")):
            if path.suffix not in SUFFIXES or "tests" in path.parts:
                continue
            for i, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
                for match in HEX.findall(line):
                    if match.lower() not in sanctioned:
                        bad.append(f"{path.relative_to(ROOT)}:{i}: {match}")

    if bad:
        print("colors outside branding/tokens.json (add a token or use one):")
        print("\n".join(f"  {b}" for b in bad))
        return 1

    check = subprocess.run(
        [sys.executable, str(ROOT / "branding/generate-tokens.py"), "--check"],
        capture_output=True, text=True,
    )
    if check.returncode != 0:
        print(check.stdout.strip())
        return 1

    print("tokens: every surface color is sanctioned; outputs in sync")
    return 0


if __name__ == "__main__":
    sys.exit(main())
