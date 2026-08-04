#!/usr/bin/env python3
"""Every app manifest must be installed where agentd actually looks.

An app becomes an agent surface by shipping a `lisa_manifest` JSON that
agentd discovers at `SYSTEM_MANIFEST_DIR` — `/usr/share/lisa/manifests`
(`daemons/agentd/src/main.rs:17`). Nothing else reads it, and agentd does
not scan for near-misses.

Issue #241 is what a one-character difference costs: Preview's manifest
installed to `/usr/share/lisa/apps/`, a directory with exactly one
reference in the repo — the line that wrote it. Preview shipped in the
image for months declaring four tools, and not one of them ever reached
the model. Nothing failed, nothing warned, nothing logged: the app works
fine for a person, so the only symptom is a silent hole in the agent
surface. A manifest in the wrong directory is indistinguishable from an
app that declares no tools at all.

So this check does not trust the destination string to be right by
review. It reads the expected directory out of agentd's own constant
(the check cannot drift from the daemon), finds every manifest in the
source tree, and asserts each one is installed there exactly — and that
no manifest-shaped file is installed anywhere else under
`/usr/share/lisa`.

Run by `just lint`; costs milliseconds and no package build.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
AGENTD_MAIN = ROOT / "daemons" / "agentd" / "src" / "main.rs"
PKGBUILD = ROOT / "os" / "packages" / "lisa" / "PKGBUILD"
# Where a first-party manifest can live in the source tree.
SOURCE_DIRS = ("apps", "shell")


def system_manifest_dir() -> str:
    """agentd's SYSTEM_MANIFEST_DIR, read from the daemon, not restated."""
    m = re.search(
        r'SYSTEM_MANIFEST_DIR:\s*&str\s*=\s*"([^"]+)"', AGENTD_MAIN.read_text()
    )
    return m.group(1) if m else ""


def manifests() -> list:
    """Source-tree files that are app manifests, by content not by name."""
    found = []
    for top in SOURCE_DIRS:
        for path in sorted((ROOT / top).glob("*/*.json")):
            try:
                doc = json.loads(path.read_text())
            except (json.JSONDecodeError, UnicodeDecodeError):
                continue
            if isinstance(doc, dict) and "lisa_manifest" in doc:
                found.append(path)
    return found


def installs() -> list:
    """(source, destination) for every `install` into $pkgdir in the PKGBUILD.

    Line continuations are joined first: the manifest installs are all
    written across two lines, so a line-at-a-time reader would see none
    of them and report success on an empty sweep.
    """
    text = PKGBUILD.read_text().replace("\\\n", " ")
    out = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("#") or not line.startswith("install "):
            continue
        # install [-Dm644 …] <src> "$pkgdir/<dest>"
        m = re.match(
            r'install\s+((?:-\S+\s+)*)(\S+)\s+"\$pkgdir(/[^"]+)"\s*$', line
        )
        if m:
            out.append((m.group(2).strip('"'), m.group(3)))
    return out


def main() -> int:
    if not AGENTD_MAIN.is_file() or not PKGBUILD.is_file():
        print(f"FAIL: expected both {AGENTD_MAIN} and {PKGBUILD}")
        return 1

    wanted = system_manifest_dir()
    if not wanted:
        print("FAIL: no SYSTEM_MANIFEST_DIR in agentd/src/main.rs — nothing to check against")
        return 1

    sources = manifests()
    pairs = installs()
    # A sweep that matched nothing must not read as success.
    if not sources:
        print(f"FAIL: found no lisa_manifest files under {'/, '.join(SOURCE_DIRS)}/ — "
              "this check is not checking anything")
        return 1
    if not pairs:
        print(f"FAIL: parsed no install lines out of {PKGBUILD} — "
              "this check is not checking anything")
        return 1

    fail = False

    # 1. Every manifest in the tree is installed, and installed to the one
    #    directory agentd reads.
    for src in sources:
        rel = src.relative_to(ROOT).as_posix()
        dests = [d for s, d in pairs if s == rel]
        if not dests:
            print(f"FAIL: {rel} is a manifest that the PKGBUILD never installs.")
            print(f"      Its tools cannot reach the model. Install it to {wanted}/.")
            fail = True
            continue
        for dest in dests:
            if str(Path(dest).parent) != wanted:
                print(f"FAIL: {rel} installs to {dest}")
                print(f"      agentd reads only {wanted} — nothing reads that path (#241).")
                fail = True

    # 2. Nothing manifest-shaped is installed elsewhere under /usr/share/lisa.
    #    Catches the same typo arriving via a file this check did not
    #    recognise as a manifest by content.
    known = {p.relative_to(ROOT).as_posix() for p in sources}
    for src, dest in pairs:
        if src in known:
            continue  # rule 1 already reported on it
        if not dest.startswith("/usr/share/lisa/") or not dest.endswith(".json"):
            continue
        if str(Path(dest).parent) != wanted:
            print(f"FAIL: {src} installs a .json to {dest}, outside {wanted}.")
            print("      If it is a manifest, nothing will read it there (#241);")
            print("      if it genuinely is not, teach this check about it.")
            fail = True

    if fail:
        return 1
    print(f"app manifests: ok ({len(sources)} installed to {wanted})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
