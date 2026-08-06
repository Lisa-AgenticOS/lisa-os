#!/usr/bin/env python3
"""No file in the signed [lisa] index may be owned by two packages.

Two packages that install the same path are not a style problem. pacman
refuses the transaction ("exists in filesystem") unless one declares a
`conflicts=` on the other, so the collision surfaces as an install
failure on somebody's device — and if the two packages are ever
installed in separate transactions, whichever ran last silently owns the
file and the other package's copy is what the machine runs.

This gate exists because of lisa-desktop#7, and because of what measuring
it found: the issue named 8 paths and one pair of packages. The index
carried **94 paths across three pairs** —

    lisa-desktop     <-> lisa-shell        51 paths
    lisa-apps        <-> lisa-shell        39 paths
    lisa-desktop-ime <-> lisa-ime           4 paths

— and not one of the five packages declared `conflicts`, `provides` or
`replaces`. Two of those pairs had never been reported by anyone.

That is the argument for reading the shipped artifacts rather than the
source: ADR-0039 split one repo into four, so no single tree contains
the whole package set any more, and a check that reads only this repo's
PKGBUILD is structurally unable to see the defect it is looking for. The
index is the one place where all four repos meet, which is exactly why
it is the one place the collision can happen.

## Running it

    # the published index — the cross-repo audit (needs network)
    python3 os/repo-tools/check-package-paths.py

    # packages just built, before they are published (offline, exact)
    python3 os/repo-tools/check-package-paths.py --pkgdir os/repo-tools/out

    # a downloaded index pair, for offline reruns
    python3 os/repo-tools/check-package-paths.py --index /tmp/lisa-index

`--explain` prints every package's file count and every overlap,
including the ones a declared `conflicts=` makes legal, so "why is this
green" has an answer that is not "trust me".

## What counts as a failure

An overlapping path fails unless one of the two packages declares a
`conflicts=` naming the other. A declared conflict is enough: pacman
will never have both installed, so no file on a real machine is ever
owned twice. It is not a fix for the duplication — it makes the
duplication loud instead of silent, which is the property this gate is
defending.

Directories are exempt. Several packages legitimately create
`/usr/share/applications`; pacman treats shared directories as normal.
Symlinks are NOT exempt: pacman conflicts on them like any other entry,
and identical symlinks in two packages is precisely the shape
lisa-desktop and lisa-shell shipped.

## What makes it go red rather than quietly green

A check that inspected nothing has not passed. Every way of ending up
with no input is a failure, never a skip:

  * an index that cannot be fetched or parsed,
  * fewer than two packages (one package cannot collide with itself, so
    a one-package set would be green forever),
  * any package whose file list is empty,
  * a `--pkgdir` with no packages in it.

That is the deletion-mutation bar the other gates here meet: delete the
input and this goes red.
"""

import argparse
import io
import shutil
import subprocess
import sys
import tarfile
import urllib.error
import urllib.request
from pathlib import Path

# The hosted [lisa] index (os/mkosi/mkosi.pkgmngr/etc/pacman.d/lisa.conf).
INDEX_URL = "https://github.com/Lisa-AgenticOS/lisa-packages/releases/download/current"
# repo-add writes both; .files carries %FILES%, .db carries %CONFLICTS%.
FILES_DB = "lisa.files.tar.gz"
DESC_DB = "lisa.db.tar.gz"

# One package cannot collide with itself. A set this small means the
# input was not what we thought it was.
MIN_PACKAGES = 2


class Fetched:
    """A package set: name -> (files, conflicts)."""

    def __init__(self, source):
        self.source = source
        self.files = {}
        self.conflicts = {}


def _strip_version(dep):
    """`gnome-shell=1:50.4-1` -> `gnome-shell`."""
    for sep in ("<=", ">=", "=", "<", ">"):
        if sep in dep:
            return dep.split(sep, 1)[0].strip()
    return dep.strip()


def _parse_repo_entry(text):
    """One repo-add desc/files blob -> {'%NAME%': [...], ...}."""
    fields, key = {}, None
    for line in text.splitlines():
        if line.startswith("%") and line.endswith("%"):
            key = line
            fields[key] = []
        elif line.strip() and key:
            fields[key].append(line.strip())
    return fields


def _read_db(blob, want):
    """Pull one field per package out of a repo-add database tarball."""
    out = {}
    with tarfile.open(fileobj=io.BytesIO(blob)) as tf:
        for member in tf.getmembers():
            if not member.name.endswith("/" + want):
                continue
            handle = tf.extractfile(member)
            if handle is None:
                continue
            fields = _parse_repo_entry(handle.read().decode("utf-8", "replace"))
            names = fields.get("%NAME%")
            # Fall back to the directory name (`pkg-1.2.3-1`) only if the
            # entry has no %NAME%, which repo-add always writes.
            name = names[0] if names else member.name.split("/")[0].rsplit("-", 2)[0]
            out[name] = fields
    return out


def from_index(location):
    """Package set from a repo-add index — a URL prefix or a local dir."""
    got = Fetched(location)
    blobs = {}
    for name in (FILES_DB, DESC_DB):
        if location.startswith("http://") or location.startswith("https://"):
            url = f"{location.rstrip('/')}/{name}"
            try:
                with urllib.request.urlopen(url, timeout=120) as response:
                    blobs[name] = response.read()
            except (urllib.error.URLError, OSError) as exc:
                raise SystemExit(
                    f"FAIL: cannot fetch {url}: {exc}\n"
                    "      The index is this gate's only input. Not being able to\n"
                    "      read it is a failure, not a reason to pass."
                )
        else:
            path = Path(location) / name
            if not path.is_file():
                raise SystemExit(
                    f"FAIL: {path} is missing — no index to check.\n"
                    "      Point --index at a directory holding "
                    f"{FILES_DB} and {DESC_DB}."
                )
            blobs[name] = path.read_bytes()

    files = _read_db(blobs[FILES_DB], "files")
    descs = _read_db(blobs[DESC_DB], "desc")
    for name, fields in files.items():
        got.files[name] = [f for f in fields.get("%FILES%", []) if not f.endswith("/")]
    for name in got.files:
        raw = descs.get(name, {}).get("%CONFLICTS%", [])
        got.conflicts[name] = {_strip_version(c) for c in raw}
    return got


def from_pkgdir(directory):
    """Package set from built .pkg.tar.* files — the pre-publish check."""
    got = Fetched(directory)
    root = Path(directory)
    if not root.is_dir():
        raise SystemExit(f"FAIL: {root} is not a directory — nothing to check.")
    packages = sorted(
        p for p in root.glob("*.pkg.tar.*") if not p.name.endswith((".sig", ".txt"))
    )
    if not packages:
        raise SystemExit(
            f"FAIL: no *.pkg.tar.* in {root} — refusing to report a clean\n"
            "      index for a directory that holds no packages."
        )
    if not shutil.which("tar"):
        raise SystemExit("FAIL: no tar on PATH; cannot read package contents.")
    for pkg in packages:
        listing = subprocess.run(
            ["tar", "-tf", str(pkg)], capture_output=True, text=True
        )
        if listing.returncode != 0:
            raise SystemExit(f"FAIL: cannot list {pkg.name}: {listing.stderr.strip()}")
        info = subprocess.run(
            ["tar", "-xOf", str(pkg), ".PKGINFO"], capture_output=True, text=True
        )
        if info.returncode != 0:
            raise SystemExit(f"FAIL: {pkg.name} has no .PKGINFO — not a pacman package.")
        name, conflicts = None, set()
        for line in info.stdout.splitlines():
            key, _, value = line.partition("=")
            key, value = key.strip(), value.strip()
            if key == "pkgname":
                name = value
            elif key == "conflict":
                conflicts.add(_strip_version(value))
        if not name:
            raise SystemExit(f"FAIL: {pkg.name} .PKGINFO declares no pkgname.")
        got.files[name] = [
            entry
            for entry in listing.stdout.splitlines()
            if entry and not entry.endswith("/") and not entry.startswith(".")
        ]
        got.conflicts[name] = conflicts
    return got


def check(got, explain):
    print(f">> package set: {got.source}")

    # -- the deletion-mutation bar ---------------------------------------
    # Everything below this point is only meaningful if there was real
    # input. Each of these is a way to end up "green" having compared
    # nothing, so each is a failure.
    if len(got.files) < MIN_PACKAGES:
        print(
            f"FAIL: {len(got.files)} package(s) in the set; need at least "
            f"{MIN_PACKAGES}.\n"
            "      A set this small cannot contain a collision, so passing it\n"
            "      would prove nothing."
        )
        return 1
    empty = sorted(name for name, files in got.files.items() if not files)
    if empty:
        print(
            "FAIL: these packages list no files at all: " + ", ".join(empty) + "\n"
            "      A package with no file list cannot be checked for overlap; a\n"
            "      truncated database must not read as a clean one."
        )
        return 1

    total = sum(len(f) for f in got.files.values())
    print(f">> {len(got.files)} packages, {total} installed paths")
    if explain:
        for name in sorted(got.files):
            declared = ", ".join(sorted(got.conflicts[name])) or "-"
            print(f"   {name:26} {len(got.files[name]):5d} files  conflicts: {declared}")

    # -- the actual property ---------------------------------------------
    owners = {}
    for name, files in got.files.items():
        for path in files:
            owners.setdefault(path.lstrip("/"), set()).add(name)

    undeclared, declared = {}, {}
    for path, pkgs in owners.items():
        if len(pkgs) < 2:
            continue
        for a in pkgs:
            for b in pkgs:
                if a >= b:
                    continue
                pair = (a, b)
                if b in got.conflicts.get(a, ()) or a in got.conflicts.get(b, ()):
                    declared.setdefault(pair, []).append(path)
                else:
                    undeclared.setdefault(pair, []).append(path)

    if explain and declared:
        print("\n-- overlaps a declared conflicts= makes unreachable --")
        for (a, b), paths in sorted(declared.items()):
            print(f"   {a} <-> {b}: {len(paths)} paths (never co-installed)")

    if not undeclared:
        print(
            f"no undeclared file collisions across {len(got.files)} packages: OK"
            + (f" ({len(declared)} pair(s) held apart by conflicts=)" if declared else "")
        )
        return 0

    print("")
    for (a, b), paths in sorted(undeclared.items(), key=lambda kv: -len(kv[1])):
        print(f"FAIL: {a} and {b} both own {len(paths)} path(s), and neither")
        print(f"      declares conflicts= on the other:")
        for path in sorted(paths)[:12]:
            print(f"        /{path}")
        if len(paths) > 12:
            print(f"        ... and {len(paths) - 12} more")
        print(
            f"      Fix by giving one package the path, or by adding\n"
            f"      conflicts=({b}) to {a} so pacman refuses loudly.\n"
        )
    return 1


def main():
    parser = argparse.ArgumentParser(
        description="Fail if two packages in the [lisa] index own the same path."
    )
    parser.add_argument(
        "--index",
        nargs="?",
        const=INDEX_URL,
        metavar="URL_OR_DIR",
        help=f"repo-add index to read (default: {INDEX_URL})",
    )
    parser.add_argument(
        "--pkgdir",
        metavar="DIR",
        help="directory of built *.pkg.tar.* to read instead of an index",
    )
    parser.add_argument(
        "--explain", action="store_true", help="print every package and every overlap"
    )
    args = parser.parse_args()

    if args.pkgdir:
        got = from_pkgdir(args.pkgdir)
    else:
        got = from_index(args.index or INDEX_URL)
    return check(got, args.explain)


if __name__ == "__main__":
    sys.exit(main())
