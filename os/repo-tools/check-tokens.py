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
# `web` joined on 2026-08-05 (ADR-0054 phase 0). The websites were the
# one surface this gate did not cover, and they had drifted exactly the
# way an ungated surface does: a cooler paper and ink than tokens.json
# sanctions, with only the violet matching by luck. That is the "fourth
# violet" defect the gate exists to prevent, living where the gate could
# not see — on the project's own marketing site.
SURFACES = ["shell", "apps", "web"]
SUFFIXES = {".js", ".css", ".vue"}
# Build output and dependencies are not authored surfaces; a hex in a
# minified vendor bundle is not a design decision anyone made.
SKIP_PARTS = {"tests", "node_modules", ".output", ".nuxt", "dist", "build"}


def main():
    tokens = json.loads((ROOT / "branding/tokens.json").read_text())
    sanctioned = {
        spec["value"].lower()
        for entries in tokens["color"].values()
        for name, spec in entries.items()
        if not name.startswith("$")
    }

    bad = []
    for surface in SURFACES:
        for path in sorted((ROOT / surface).rglob("*")):
            if path.suffix not in SUFFIXES or SKIP_PARTS & set(path.parts):
                continue
            for i, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
                for match in HEX.findall(line):
                    if match.lower() not in sanctioned:
                        bad.append(f"{path.relative_to(ROOT)}:{i}: {match}")

    if bad:
        print("colors outside branding/tokens.json (add a token or use one):")
        print("\n".join(f"  {b}" for b in bad))
        return 1

    # The account palette is COPIED into apps/mail/lib/rail.js rather than
    # imported: branding/out/ is not staged into the apps payload, so an
    # import resolves on a dev host and throws on a device. Membership —
    # what the loop above checks — is not enough for a copy that is
    # INDEXED into: an account's colour comes from `ACCENTS[h % length]`,
    # so a reordered or short list silently recolours every account,
    # which is the one thing #248 said must never happen. Assert the copy
    # equals the group, in order.
    if (rail := ROOT / "apps/mail/lib/rail.js").exists():
        want = [
            spec["value"].lower()
            for name, spec in tokens["color"]["account"].items()
            if not name.startswith("$")
        ]
        block = re.search(r"export const ACCENTS = \[(.*?)\]", rail.read_text(), re.S)
        got = [h.lower() for h in HEX.findall(block.group(1))] if block else []
        if got != want:
            print("apps/mail/lib/rail.js ACCENTS does not match color.account "
                  "in branding/tokens.json (#248).")
            print(f"  tokens.json: {want}")
            print(f"  rail.js:     {got}")
            print("  The list is INDEXED into, so order and length decide which "
                  "account gets which colour.")
            return 1

    check = subprocess.run(
        [sys.executable, str(ROOT / "branding/generate-tokens.py"), "--check"],
        capture_output=True, text=True,
    )
    if check.returncode != 0:
        print(check.stdout.strip())
        return 1

    print("tokens: every surface color is sanctioned; account palette in sync; outputs in sync")
    return 0


if __name__ == "__main__":
    sys.exit(main())
