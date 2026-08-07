#!/usr/bin/env python3
"""Every import an installed extension makes must resolve in the farm (#333).

GNOME Shell loads system extensions through
/usr/share/gnome-shell/extensions/<uuid> symlinks, and GJS resolves a
relative import against the path a module was imported AS — the symlink
path, not its target. So `../../lisa.sdk/bus/proxy.js` inside an
extension does not land beside the payload tree; it lands beside the
uuid links, in the farm itself. v20260807.85 shipped with the assistant
dead because the farm had no lisa.sdk entry: every unit test was green,
the payload check was green, and the one path shape that mattered — the
one the compositor actually uses — was checked nowhere.

This gate replays that path shape statically, on every commit:

1. read os/packages/lisa/PKGBUILD and collect the farm — every
   `ln -s /usr/share/lisa/shell/<src> …/gnome-shell/extensions/<name>`;
2. map each farm entry back to its repo source tree (shell/<src>;
   lisa.sdk lives at apps/lisa.sdk — see build-apps-payload.sh);
3. for every .js file in every farm entry, resolve each relative import
   the way GJS will — against the file's FARM path — and require the
   result to stay inside an installed farm entry and name a file that
   exists in that entry's repo tree.

Deleting the lisa.sdk symlink from the PKGBUILD, renaming an entry, or
importing one directory deeper than the farm provides: each goes red
here instead of on a shipped desktop.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
PKGBUILD = REPO / "os/packages/lisa/PKGBUILD"

# Where a payload directory's source lives in this repo. Everything the
# PKGBUILD links out of /usr/share/lisa/shell/<src> is staged by
# build-apps-payload.sh from one of these roots.
def repo_source(src: str) -> Path:
    if src == "lisa.sdk":
        return REPO / "apps" / "lisa.sdk"
    return REPO / "shell" / src


def farm_from_pkgbuild(text: str) -> dict[str, str]:
    """farm name -> payload source dir, from the PKGBUILD's ln -s lines.

    The PKGBUILD writes them as a two-line continuation:
        ln -s /usr/share/lisa/shell/<src> \\
            "$pkgdir/usr/share/gnome-shell/extensions/<name>"
    """
    joined = text.replace("\\\n", " ")
    farm: dict[str, str] = {}
    for m in re.finditer(
        r"ln\s+-s\s+/usr/share/lisa/shell/(\S+)\s+"
        r"\"\$pkgdir/usr/share/gnome-shell/extensions/([^\"]+)\"",
        joined,
    ):
        farm[m.group(2)] = m.group(1)
    return farm


IMPORT_RE = re.compile(
    r"""(?:^\s*import\b[^'"]*|^\s*export\b[^'"]*\bfrom\s*|\bimport\s*\(\s*)
        ['"]([^'"]+)['"]""",
    re.MULTILINE | re.VERBOSE,
)


def check() -> list[str]:
    farm = farm_from_pkgbuild(PKGBUILD.read_text())
    errors: list[str] = []
    if "lisa.sdk" not in farm:
        errors.append(
            f"{PKGBUILD.relative_to(REPO)}: the farm has no lisa.sdk entry — "
            "the overlay extension dies at import on a shipped image "
            "(v20260807.85, #333)"
        )
    for name, src in sorted(farm.items()):
        tree = repo_source(src)
        if not tree.is_dir():
            errors.append(
                f"{PKGBUILD.relative_to(REPO)}: extensions/{name} points at "
                f"/usr/share/lisa/shell/{src}, and no source for it exists "
                f"at {tree.relative_to(REPO)}"
            )
            continue
        for js in sorted(tree.rglob("*.js")):
            if "node_modules" in js.parts or "tests" in js.parts:
                continue
            farm_path = Path("farm") / name / js.relative_to(tree)
            for spec in IMPORT_RE.findall(js.read_text(errors="replace")):
                if not spec.startswith("."):
                    continue  # gi://, resource://, bare — not path-resolved
                resolved = (farm_path.parent / spec).resolve().relative_to(
                    Path("farm").resolve()
                ) if (farm_path.parent / spec).resolve().is_relative_to(
                    Path("farm").resolve()
                ) else None
                if resolved is None:
                    errors.append(
                        f"{js.relative_to(REPO)}: '{spec}' escapes the "
                        "extension farm entirely — nothing is installed there"
                    )
                    continue
                entry, *rest = resolved.parts
                if entry not in farm:
                    errors.append(
                        f"{js.relative_to(REPO)}: '{spec}' resolves to farm "
                        f"entry '{entry}', which the PKGBUILD does not "
                        "install — this is the #333 defect"
                    )
                    continue
                target = repo_source(farm[entry]).joinpath(*rest)
                if not target.exists():
                    errors.append(
                        f"{js.relative_to(REPO)}: '{spec}' names "
                        f"{'/'.join(rest)} inside {entry}, which does not "
                        f"exist in {repo_source(farm[entry]).relative_to(REPO)}"
                    )
    return errors


def main() -> int:
    errors = check()
    for e in errors:
        print(f"error: {e}", file=sys.stderr)
    if errors:
        print(f"\n{len(errors)} extension import(s) would die on a shipped "
              "image (#333)", file=sys.stderr)
        return 1
    print("check-extension-imports: every farm import resolves")
    return 0


if __name__ == "__main__":
    sys.exit(main())
