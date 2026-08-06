#!/usr/bin/env python3
"""A derived PKGBUILD must still match what it was derived FROM (#284).

WHY THIS EXISTS. `os/packages/lisa-desktop-control-center/PKGBUILD` is
Arch's `gnome-control-center` PKGBUILD plus a Lisa delta. That makes it
two things in one file: fields that are OURS (pkgname, provides,
conflicts, replaces, the appended sources) and fields that are ARCH'S,
copied by hand and expected to track upstream exactly.

Nothing checked the second kind. Bumping the fork 50.3 -> 50.4 therefore
went like this:

  1. change pkgver          -> push -> ~10 min -> red: "b2sums FAILED"
  2. change b2sums[0]       -> push -> ~10 min -> ...

Each round trip discovers ONE coupled field, because makepkg reports the
first thing that stops it and nothing enumerates the rest. That is not
bad luck, it is the absence of a check: the stock half of the file is a
hand-maintained copy of a moving target with no diff against the target.

WHAT THIS DOES. Arch's PKGBUILD is vendored beside ours at a pinned
commit (`upstream.PKGBUILD`). This compares the two OFFLINE, so the
whole coupled set is reported at once, before a push:

  pkgver, arch, license, url, options, validpgpkeys
        must MATCH (arch= may add architectures we build for)
  depends, makedepends, checkdepends
        Arch's must be a SUBSET of ours — a fork may add, never drop,
        because dropping one is a runtime failure nobody sees at build
  source
        Arch's must be a PREFIX of ours — our local files are appended,
        so upstream's entries keep their indices
  b2sums
        must be the same length as source, and its first len(arch)
        entries must equal Arch's. THIS IS THE ONE THAT SHIPPED: a
        b2sum is positionally bound to a source, and pkgver moves the
        first entry because it pins the tag. Change one without the
        other and makepkg refuses the build.

`--refresh` re-fetches Arch's current PKGBUILD (the only networked
mode) and rewrites the vendored copy, printing the diff. That is the
rebase workflow: refresh, read what moved, change our file once.

WHAT THIS DOES NOT DO. It says nothing about whether the Lisa delta
still APPLIES — whether GNOME moved an anchor prepare() greps for. That
question needs the upstream source tree, so it belongs where the source
is: the nine guards in prepare(), which fail the build by name. Two
different checks for two different failures; neither substitutes for
the other, and this file should not pretend to cover both.
"""

from __future__ import annotations

import argparse
import difflib
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PKG_DIR = ROOT / "os" / "packages"

MUST_MATCH = ("pkgver", "license", "url", "options", "validpgpkeys")
SUPERSET = ("arch", "depends", "makedepends", "checkdepends")


def _array(text: str, field: str) -> list[str] | None:
    """A PKGBUILD array as an ordered list, or None when absent.

    Scans to the MATCHING close paren rather than regex-ing to the next
    one. A non-greedy `\\((.*?)\\)` reads `license=(GPL-2.0-or-later)`
    correctly and `depends=(\\n  a\\n  b\\n)` correctly, but the two
    cannot share one pattern: anchoring the close to a line start breaks
    the first, and not anchoring it breaks nothing here yet — which is
    the trap, because it breaks silently later on the first array whose
    entry contains a paren. Balance-scanning is the same amount of code
    and has no such day.
    """
    m = re.search(rf"^{field}\+?=\(", text, re.M)
    if m is None:
        return None
    i, depth, quote, buf = m.end(), 1, "", []
    while i < len(text) and depth:
        c = text[i]
        if quote:
            if c == quote:
                quote = ""
        elif c in "\"'":
            quote = c
        elif c == "#" and (not buf or buf[-1] in " \t\n"):
            # comment to end of line
            j = text.find("\n", i)
            i = len(text) if j < 0 else j
            continue
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if not depth:
                break
        buf.append(c)
        i += 1
    inner = "".join(buf)
    return [t.strip("\"'") for t in re.findall(r'"[^"]*"|\'[^\']*\'|\S+', inner)]


def _scalar(text: str, field: str) -> str | None:
    m = re.search(rf'^{field}=(?!\()"?([^"\n]*)"?', text, re.M)
    return m.group(1).strip() if m else None


def _strip_comments(text: str) -> str:
    """Drop whole-line comments so a vendored header cannot be parsed."""
    return "\n".join(l for l in text.splitlines() if not l.lstrip().startswith("#"))


def _get(text: str, field: str):
    return _array(text, field) if _array(text, field) is not None else _scalar(text, field)


def check(fork: Path, upstream: Path) -> list[str]:
    rel = fork.relative_to(ROOT)
    ours = _strip_comments(fork.read_text())
    theirs = _strip_comments(upstream.read_text())
    errors: list[str] = []

    for field in MUST_MATCH:
        a, b = _get(theirs, field), _get(ours, field)
        if a is None and b is None:
            continue
        if a != b:
            errors.append(
                f"{rel}: {field} is {b!r} but the PKGBUILD it derives from "
                f"has {a!r}. Either take upstream's value or record why ours "
                f"differs — a silent divergence here is a fork nobody decided to make."
            )

    for field in SUPERSET:
        a, b = _get(theirs, field), _get(ours, field)
        if a is None:
            continue
        missing = [x for x in a if x not in (b or [])]
        if missing:
            errors.append(
                f"{rel}: {field} is missing upstream entries {missing}. A fork "
                f"may ADD to these, never drop: a dropped depends does not fail "
                f"the build, it fails at runtime on somebody's machine."
            )

    src_a, src_b = _get(theirs, "source") or [], _get(ours, "source") or []
    if src_b[: len(src_a)] != src_a:
        diff = [
            f"      [{i}] upstream {src_a[i]!r}\n          ours     "
            f"{src_b[i] if i < len(src_b) else '<missing>'!r}"
            for i in range(len(src_a))
            if i >= len(src_b) or src_b[i] != src_a[i]
        ]
        errors.append(
            f"{rel}: upstream's source=() is not a prefix of ours. Our local "
            f"files must be APPENDED, because b2sums is positional — inserting "
            f"one shifts every checksum after it onto the wrong file.\n"
            + "\n".join(diff)
        )

    sum_a, sum_b = _get(theirs, "b2sums") or [], _get(ours, "b2sums") or []
    if len(sum_b) != len(src_b):
        errors.append(
            f"{rel}: {len(src_b)} sources but {len(sum_b)} b2sums. makepkg "
            f"pairs them by index; a short array silently leaves the tail "
            f"unverified."
        )
    for i, want in enumerate(sum_a):
        got = sum_b[i] if i < len(sum_b) else None
        if got != want:
            errors.append(
                f"{rel}: b2sums[{i}] does not match upstream's for the same "
                f"source ({src_a[i] if i < len(src_a) else '?'}).\n"
                f"      upstream {want}\n      ours     {got}\n"
                f"      Entry 0 pins the git TAG, so it moves with pkgver — "
                f"this is exactly the pair that cost a CI round trip on the "
                f"50.3 -> 50.4 bump."
            )
    return errors


def pairs() -> list[tuple[Path, Path]]:
    return [
        (d / "PKGBUILD", d / "upstream.PKGBUILD")
        for d in sorted(PKG_DIR.iterdir())
        if (d / "upstream.PKGBUILD").exists() and (d / "PKGBUILD").exists()
    ]


def refresh() -> int:
    """Re-fetch each vendored upstream PKGBUILD and show what moved."""
    rc = 0
    for fork, up in pairs():
        text = up.read_text()
        m = re.search(r"^#\s+repo:\s+(\S+)", text, re.M)
        if not m:
            print(f"{up}: no `# repo:` provenance line — cannot refresh", file=sys.stderr)
            rc = 1
            continue
        repo = m.group(1)
        proj = repo.split("gitlab.archlinux.org/")[-1].replace("/", "%2F")
        api = (
            f"https://gitlab.archlinux.org/api/v4/projects/{proj}"
            f"/repository/commits?ref_name=main&per_page=1"
        )
        head = subprocess.run(
            ["curl", "-fsSL", api], capture_output=True, text=True, check=True
        ).stdout
        sha = re.search(r'"id":"([0-9a-f]{40})"', head).group(1)
        title = re.search(r'"title":"(.*?)"', head).group(1)
        body = subprocess.run(
            ["curl", "-fsSL", f"{repo}/-/raw/{sha}/PKGBUILD"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        old_body = "\n".join(
            l for l in text.splitlines() if not l.startswith("# ")
        ).lstrip("\n")
        if old_body.strip() == body.strip():
            print(f"{up.parent.name}: already at upstream {title} ({sha[:8]})")
            continue
        print(f"{up.parent.name}: upstream moved to {title} ({sha[:8]})")
        for line in difflib.unified_diff(
            old_body.splitlines(), body.splitlines(), "vendored", "upstream", lineterm=""
        ):
            print(f"  {line}")
        rc = 1
    return rc


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--refresh",
        action="store_true",
        help="fetch upstream's current PKGBUILD and show the diff (networked)",
    )
    args = ap.parse_args()

    if args.refresh:
        return refresh()

    found = pairs()
    if not found:
        # A sweep that matched nothing must not read as a pass.
        print(
            "check-fork-derivation: no PKGBUILD/upstream.PKGBUILD pairs found "
            "— refusing to pass vacuously",
            file=sys.stderr,
        )
        return 1

    errors: list[str] = []
    for fork, up in found:
        errors += check(fork, up)

    if errors:
        print("check-fork-derivation: FAIL", file=sys.stderr)
        for e in errors:
            print(f"  {e}", file=sys.stderr)
        return 1

    print(f"check-fork-derivation: {len(found)} fork(s) match their upstream: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
