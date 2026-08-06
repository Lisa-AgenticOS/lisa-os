#!/usr/bin/env python3
"""Every package in a Lisa Desktop image is somebody's decision (#284).

The reference iMac installs 672 packages and, until this file existed,
nobody could say which of them are ours by intent, which are stock by
intent, and which are simply along for the ride. Three of the four
desktop surprises found in the first week of August 2026 have that one
shape — **something arrives, changes, or gets outranked without any
decision being visible**:

  * `gnome-control-center 50.3-2` and `gnome-keybindings 50.3-2` on the
    device are OUR forks wearing the STOCK names, winning only by
    pkgrel. `vercmp 50.3-2 50.4-1` is -1 and Arch already ships 50.4-1
    with 51beta staged, so the next image build silently ships stock and
    the Intelligence panel (ADR-0012) disappears. That is #284, and
    CLAUDE.md's fork rule exists because the same race already ran once,
    on 2026-08-04.
  * `gnome-online-accounts 3.58.1-1` is on the device although
    `lisa-desktop-online-accounts` is named in the release lane's
    `Packages=`. The fork is declared and absent; the stock one is
    present and undeclared. Nothing said so.
  * `nautilus`, `curl` and `gst-plugins-base` each arrived — or failed
    to arrive — as somebody else's transitive dependency. mkosi.conf now
    names all three, each with a paragraph of regret. `gjs`, `gtk4`,
    `libadwaita` and `mutter` are still riding in that way today, and
    they are the toolkit, the compositor and the language every Lisa
    surface is written in.

By contrast `lisa-desktop-shell` is safe permanently, because it
replaces by CONTRACT: a different package NAME carrying
`provides=(gnome-shell=…)` and `conflicts=(gnome-shell)`. Arch can ship
gnome-shell 52 tomorrow and nothing displaces it. That is the property
this file polices — CLAUDE.md, repo mechanics: *"Fork packages replace
stock by contract, never by name."*

--------------------------------------------------------------------
WHAT IS DISCOVERED, AND WHAT IS DECLARED

Following check-egress-units.py: DISCOVER the population, key on the
PROPERTY rather than on a filename, and make an unclassified
Lisa-adjacent package an ERROR.

Discovered from the tree, never typed:

  1. Every package this repo BUILDS — `os/packages/**/PKGBUILD`, read
     for `pkgbase=`, `pkgname=` (scalar and array), and the `provides=`
     / `conflicts=` / `replaces=` of both the global scope and every
     `package_<name>()` body.
  2. Every package any lane DECLARES — the `Packages=` lists in
     `os/mkosi/mkosi.conf`, `os/mkosi/mkosi.conf.d/*.conf`, and the
     `50-release.conf` heredoc inside `.github/workflows/release.yml`.
     A name in a `Packages=` list is a decision recorded in git; a name
     that reaches the image any other way is not.
  3. What an image ACTUALLY got — `pacman -Q` output at
     `/usr/lib/lisa/packages.manifest`, the same source check-desktop.sh
     reads from the built root.

Two things cannot be discovered offline and are therefore DECLARED
below, each with a reason:

  * whether Arch ships a package by a given name (so PORTS has to be a
    list — see its note), and
  * which stock components are FOUNDATION and which are INTERIM. That
    is not a fact about the tree, it is the decision ADR-0058 records.

--------------------------------------------------------------------
THE BUCKETS

  ours        a `lisa-*` package we build, replacing nothing
  fork        a `lisa-*` package that replaces a stock one BY CONTRACT
              (provides= + conflicts= on the stock name)
  port        upstream software packaged under its own upstream name
              because Arch ships none — llama.cpp, whisper.cpp, piper,
              cyrus-sasl-xoauth2
  foundation  deliberately stock and never to be forked (CLAUDE.md
              rule 11): the toolkit, the compositor, the session, the
              portals, the IME framework, the GJS runtime
  interim     stock because the Lisa equivalent does not exist yet
              (ADR-0048's honest interim): Files, the terminal, the
              keyring, the greeter
  transitive  everything else — a dependency nobody chose directly.
              NOT POLICED, and that is a decision, not an oversight:
              see "What is deliberately unpoliced" below.

--------------------------------------------------------------------
WHAT MAKES IT GO RED

Repo half (runs with no manifest at all):

  R1  A package this repo builds that is neither `lisa-*` nor a
      declared PORT. **This is #284's source shape**: a PKGBUILD with
      `pkgbase=gnome-control-center` producing `gnome-control-center`.
  R2  A package that claims a stock name with `provides=` but not
      `conflicts=` (co-installable, two owners for every path — the
      lisa-desktop#7 shape), or with `conflicts=` but not `provides=`
      (every dependency on the stock name becomes unsatisfiable).
  R3  A FOUNDATION name that anything forks, or that a PKGBUILD here
      produces. CLAUDE.md rule 11, mechanized.
  R4  A name in two of FOUNDATION / INTERIM / PORTS, or a
      FOUNDATION/INTERIM name that a fork already replaces.
  R5  A `lisa-*` name declared in a lane's `Packages=` that nothing
      here builds and no OUT_OF_TREE record claims — the unclassified
      Lisa-adjacent package.
  R6  Discovery that found no PKGBUILDs, no forks, or no declared
      lanes. A matched-nothing sweep fails: an `os/packages` refactor
      must not turn this file into a green no-op.
  R7  A FOUNDATION or INTERIM package that no lane declares, unless it
      is named in UNDECLARED_DEBT under its ceiling (see there).

Manifest half (`--manifest FILE --lane LANE`):

  M1  A stock name that one of our forks replaces is INSTALLED —
      unless that lane's own `Packages=` declares the stock name, which
      makes it a decision in git (the aarch64 lane and stock
      gnome-shell, ADR-0021). Today, on the device: `gnome-control-
      center`, `gnome-keybindings`, `gnome-online-accounts`. #284.
  M2  A fork declared for this lane is MISSING from the image.
  M3  An `ours` or `port` package declared for this lane is MISSING.
      Absence fails; it does not pass quietly.
  M4  A FOUNDATION or INTERIM package is MISSING. The compositor and
      the toolkit are not optional, and neither is the honest interim.
  M5  A `lisa`-namespace package in the image that discovery cannot
      account for.
  M6  The lane the caller claims contradicts the manifest — `aarch64`
      without `linux-aarch64`, an x86_64 lane without `linux`.
  M7  A manifest that is unreadable, empty, a directory, or yields zero
      package rows. A gate with nothing to check must not pass; that is
      #297's lesson, one file over.

--------------------------------------------------------------------
WHAT IS DELIBERATELY UNPOLICED, AND WHY

`transitive` is not checked at all, and there is no attempt to
classify 600-odd rows. Hand-classifying them would be inventing
classifications, which CLAUDE.md rule 10 forbids more strongly than it
asks for coverage — a table asserting that `libxkbcommon` is
"foundation" would be a fact nobody established, aging badly and
teaching readers that the other rows are guesses too.

What replaces that coverage is the ratchet in UNDECLARED_DEBT: any
package this file DOES name must be declared by a lane, so the set of
things "along for the ride" that we depend on is finite, written down,
and cannot grow for free.

--------------------------------------------------------------------
Usage:
    check-desktop-inventory.py                      # the repo half
    check-desktop-inventory.py --explain            # the bucket table
    check-desktop-inventory.py --manifest M --lane release
    check-desktop-inventory.py --list fork|ours|port|foundation|interim
    check-desktop-inventory.py --root DIR           # judge another tree
"""

import re
import sys
from pathlib import Path

# ------------------------------------------------------------- declared
#
# FOUNDATION — stock, and never to be forked. CLAUDE.md rule 11:
# "GTK4/libadwaita and Mutter are never forked: toolkit and compositor
# are foundation, not experience." The rest of this set is os/mkosi/
# mkosi.conf's own sentence, quoted rather than invented: "GNOME's
# *foundation* stays and is not up for removal — mutter, GTK4/libadwaita,
# gnome-session, gnome-settings-daemon, gsettings-desktop-schemas, the
# portals."
#
# An entry here is a promise with teeth: R3 fails if anything forks it,
# and M4 fails if it goes missing from an image.
FOUNDATION = {
    "mutter": (
        "The compositor. ADR-0038 forked the Shell and explicitly did "
        "NOT fork Mutter — the Shell is a Mutter plugin, and rebase cost "
        "scales with the width of the delta."
    ),
    "gtk4": (
        "The toolkit. CLAUDE.md rule 11 by name; every Lisa app is a "
        "GTK4 app (ADR-0047)."
    ),
    "libadwaita": (
        "The widget layer. Same rule, same sentence — and ADR-0056 is "
        "explicit that lisa_ui is a DIALECT on top of libadwaita, not a "
        "replacement for it."
    ),
    "gjs": (
        "The language runtime. ADR-0047 made GJS/GTK4 the one toolkit "
        "and parked Flutter; the Shell surfaces, the IME bridge and "
        "every first-party app are GJS. Forking it would be forking the "
        "interpreter our supply-chain story rests on."
    ),
    "gnome-session": "Session manager. mkosi.conf names it foundation.",
    "gnome-settings-daemon": "Session daemons. mkosi.conf names it foundation.",
    "gsettings-desktop-schemas": (
        "The schema set every surface reads. mkosi.conf names it "
        "foundation; lisa-shell ships an OVERRIDE into it "
        "(10_lisa-shell.gschema.override), which is the opposite of a "
        "fork and only works while the stock schemas are there."
    ),
    "xdg-desktop-portal-gnome": (
        "The portal backend. mkosi.conf: 'the portals'. Lisa ADDS a "
        "portal (xdg-desktop-portal-lisa, PLAN §5.5) beside it and "
        "replaces neither."
    ),
    "xdg-desktop-portal-gtk": "The portal fallback (file chooser et al.).",
    "fcitx5": (
        "The input-method framework. PLAN §5.7.3 layer 2 makes "
        "fcitx5-lisa an ADDON hosted by it — the same extend-never-fork "
        "relationship GTK4 has, so it belongs in the same bucket."
    ),
}

# INTERIM — stock because the Lisa equivalent does not exist yet.
# ADR-0048: "Where a Lisa app does not exist, ship the stock GNOME app
# *unpatched* — that is the honest interim, not a gap to close with a
# patch set."
#
# The difference from FOUNDATION is the direction of travel, and it is
# load-bearing: forking a FOUNDATION entry is a rule-11 violation, while
# replacing an INTERIM entry (by contract, with a `lisa-*` name) is the
# plan. Moving a name between these two dicts is an ADR, not an edit.
INTERIM = {
    "nautilus": (
        "Files. `apps/files` is a README and nothing else (CLAUDE.md "
        "component map: 'not started'). Preview implements "
        "org.gnome.NautilusPreviewer2 so Space gives Quick Look (#146), "
        "which integrates with nothing if Nautilus is absent — mkosi.conf "
        "declares it for exactly that reason."
    ),
    "gnome-console": (
        "The terminal. §5.8 wants the `lisa` CLI preinstalled and a CLI "
        "needs a terminal to live in; no Lisa terminal exists."
    ),
    "gnome-keyring": (
        "The Secret Service. Without it a connected Online Account has "
        "no token to hand out — measured on the field iMac 2026-07-30. "
        "No Lisa credential store exists for the SESSION (remoted keeps "
        "provider keys in its own store, deliberately)."
    ),
    "gdm": (
        "The greeter. ADR-0035 wants the desktop to be a prompt from "
        "first pixel; nothing implements a Lisa greeter, so the login "
        "path is stock and honest about it."
    ),
}

# PORTS — upstream software packaged under its OWN upstream name because
# Arch ships no package by that name.
#
# This one HAS to be a list, and the reason is worth stating: the fact
# that separates a port from a fork-by-name is "does Arch ship a package
# called X", and that is a question about a package database this gate
# cannot reach (and must not need to reach — a gate that fails without
# the network is a gate people learn to skip). So the check is inverted:
# any foreign-named package this repo builds MUST be named here with a
# reason, and one that is not fails R1. `gnome-control-center` built
# under the stock name would fail here, which is #284 caught at its
# source rather than on a device.
PORTS = {
    "llama.cpp": (
        "PLAN §5.1's inference engine. Not in Arch's repos; built here "
        "with the Vulkan backend (#193) and pinned in ports.lock."
    ),
    "whisper.cpp": "PLAN §5.7.5, the ear. Not in Arch's repos.",
    "piper": "PLAN §5.7.5, the mouth. Not in Arch's repos.",
    "cyrus-sasl-xoauth2": (
        "The XOAUTH2 SASL plugin mbsync needs for a Google account "
        "(#155). Neither Arch nor upstream Cyrus SASL ships it, which is "
        "the whole reason mail sync could not work without this package."
    ),
}

# Lisa packages built by ANOTHER repo (ADR-0039). Their PKGBUILDs are not
# in this tree, so their provides/conflicts cannot be read here — the
# contract is asserted against the image instead (M1/M2), and the version
# pin is check-desktop.sh's job (#273). Recorded rather than silent:
# without this, `lisa-desktop-shell` in a manifest would fail M5 as an
# unclassified Lisa package, and `gnome-shell` would be invisible.
OUT_OF_TREE = {
    "lisa-desktop-shell": {
        "repo": "lisa-desktop",
        "replaces": "gnome-shell",
        "why": (
            "ADR-0038's hard fork of GNOME Shell. It is the WORKED "
            "EXAMPLE this whole file generalises: a different package "
            "name carrying provides=(gnome-shell=…) conflicts=(gnome-"
            "shell), so Arch can ship gnome-shell 52 and nothing "
            "displaces it. Its PKGBUILD lives in the lisa-desktop repo; "
            "os/mkosi/desktop.lock pins the artifact."
        ),
    },
}

# ------------------------------------------------------------- the debt
#
# RATCHETED, like check-egress-units.py's DEBT_CEILING and for the same
# reason: this dict SUPPRESSES a finding, so adding to it must not be a
# free, green, one-line change. `len()` is asserted against the ceiling,
# so an entry needs a second line raising it — and that line reads, in
# the diff, as "the number of load-bearing packages that reach the image
# by accident goes from N to N+1", which is not a sentence anyone lands
# without noticing.
#
# Every ceiling is a number that may only go DOWN without argument.
DEBT_CEILING = {"UNDECLARED_DEBT": 6}

# FOUNDATION/INTERIM packages that reach the image only because
# something else happens to depend on them. Each is #45's shape exactly
# ("libcurl arrived only as an accidental transitive dependency") applied
# to the toolkit, the compositor and the runtime.
#
# They are NOT fixed here: os/mkosi/** is another change's territory, and
# a gate that edits the thing it judges is not a gate. Naming them is the
# whole contribution — before this dict, the fact that Lisa's compositor
# and interpreter are undeclared was not written down anywhere.
UNDECLARED_DEBT = {
    "mutter": "arrives as a dependency of the shell package. #300.",
    "gtk4": "arrives as a dependency of most of the session. #300.",
    "libadwaita": "same. #300.",
    "gjs": (
        "arrives as a dependency of gnome-shell/lisa-desktop-shell — the "
        "runtime every Lisa surface is written in, declared by nobody. "
        "#300."
    ),
    "gsettings-desktop-schemas": (
        "arrives transitively, and lisa-shell installs an OVERRIDE into "
        "it. #300."
    ),
    "gnome-settings-daemon": (
        "The sharpest one, and found by this check rather than by a "
        "person: os/mkosi/mkosi.conf NAMES it in prose — 'GNOME's "
        "*foundation* stays and is not up for removal — mutter, "
        "GTK4/libadwaita, gnome-session, gnome-settings-daemon, "
        "gsettings-desktop-schemas, the portals' — and then does not put "
        "it in Packages=. gnome-session IS declared, three lines from "
        "the same sentence. #300."
    ),
}

# Lane -> the config files whose Packages= lists that lane resolves.
# `release` is the only lane that carries the Lisa packages: they live in
# release.yml's generated 50-release.conf, which is why a nightly image
# has none and why a lane has to be stated rather than guessed.
LANES = {
    "nightly": ("os/mkosi/mkosi.conf", "os/mkosi/mkosi.conf.d/x86_64.conf"),
    "release": ("os/mkosi/mkosi.conf", "os/mkosi/mkosi.conf.d/x86_64.conf",
                ".github/workflows/release.yml"),
    "aarch64": ("os/mkosi/mkosi.conf", "os/mkosi/mkosi.conf.d/aarch64.conf"),
}

# The one fact in a manifest that says which architecture built it —
# check-desktop.sh reads the same pair for the same reason.
LANE_KERNEL = {"nightly": "linux", "release": "linux", "aarch64": "linux-aarch64"}

# A name in OUR namespace. ADR-0016 fixed the reverse-DNS prefixes; the
# package namespace is the flat half of the same decision, and it is what
# lets `conflicts=(lisa-desktop lisa-apps)` be read as "two Lisa packages
# that must not co-install" rather than as a fork of something stock.
def is_lisa_name(name: str) -> bool:
    return name == "lisa" or name.startswith("lisa-")


# A `provides=` entry that is a shared-object name, not a package name:
# lisa-desktop-online-accounts provides libgoa-1.0.so, which is a soname
# guarantee and not a claim on anybody's package.
def is_soname(name: str) -> bool:
    return ".so" in name


# ---------------------------------------------------------- PKGBUILD read

ASSIGN = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\+?=(.*)$", re.S)
VAR = re.compile(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}|\$([A-Za-z_][A-Za-z0-9_]*)")
PKGFUNC = re.compile(r"^package_([A-Za-z0-9_.+-]+)\s*\(\)")


def _expand(word, env):
    def sub(match):
        return env.get(match.group(1) or match.group(2), match.group(0))
    return VAR.sub(sub, word)


def _values(raw, env):
    """The words an assignment's right-hand side names.

    Arrays (`(a b c)`) and scalars both, quotes stripped, `$var`
    expanded. A word that still holds a `$` after expansion is returned
    as-is so the caller can see it rather than lose it.
    """
    raw = raw.strip()
    if raw.startswith("(") and raw.endswith(")"):
        raw = raw[1:-1]
    out = []
    for word in raw.split():
        word = _expand(word, env).strip("\"'")
        if word:
            out.append(word)
    return out


def _joined(text):
    """(line number, logical line) with array literals and backslash
    continuations joined, so a multi-line `pkgname=(\\n a\\n b\\n)` is one
    assignment rather than three lines of noise."""
    out, buf, start, depth = [], "", 1, 0
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0] if raw.lstrip().startswith("#") else raw
        if not buf:
            start = n
        cont = line.endswith("\\")
        if cont:
            line = line[:-1]
        buf += line + " "
        depth += line.count("(") - line.count(")")
        if cont or (depth > 0 and ASSIGN.match(buf.strip())):
            continue
        out.append((start, buf.strip()))
        buf, depth = "", 0
    if buf.strip():
        out.append((start, buf.strip()))
    return out


class Pkg:
    """One package a PKGBUILD produces."""

    __slots__ = ("name", "pkgbuild", "provides", "conflicts", "replaces")

    def __init__(self, name, pkgbuild):
        self.name = name
        self.pkgbuild = pkgbuild
        self.provides, self.conflicts, self.replaces = [], [], []

    def claims(self):
        """Foreign (non-`lisa-*`, non-soname) names this package claims."""
        names = set()
        for entry in self.provides + self.conflicts + self.replaces:
            base = entry.split("=")[0].split("<")[0].split(">")[0]
            if base and not is_lisa_name(base) and not is_soname(base):
                names.add(base)
        return names

    def kind(self, which):
        return {"provides": self.provides, "conflicts": self.conflicts,
                "replaces": self.replaces}[which]


def read_pkgbuild(root, path, errors):
    """Every package `path` produces, with its provides/conflicts/replaces.

    Both scopes are read: the global one and each `package_<name>()`
    body. gnome-control-center's fork declares its contract ONLY inside
    the two package functions, so a parser that read the global scope
    alone would report the forks as declaring nothing — a false red that
    teaches people to delete the check.
    """
    text = path.read_text()
    rel = str(path.relative_to(root))
    lines = _joined(text)

    env, names, order = {}, [], []
    for _n, line in lines:
        match = ASSIGN.match(line)
        if not match:
            continue
        key, raw = match.group(1), match.group(2)
        if key in ("pkgname", "pkgbase", "provides", "conflicts", "replaces"):
            continue
        vals = _values(raw, env)
        if len(vals) == 1:
            env[key] = vals[0]

    pkgbase = None
    for _n, line in lines:
        match = ASSIGN.match(line)
        if not match:
            continue
        key, raw = match.group(1), match.group(2)
        if key == "pkgbase":
            got = _values(raw, env)
            pkgbase = got[0] if got else None
        elif key == "pkgname":
            for got in _values(raw, env):
                if got not in order:
                    order.append(got)
    if not order and pkgbase:
        order = [pkgbase]
    if not order:
        errors.append(
            f"{rel}: produces no package name this check can read — no "
            f"`pkgname=` and no `pkgbase=`. A PKGBUILD whose output is "
            f"unknown is a package nothing below classifies."
        )
        return []

    pkgs = {name: Pkg(name, rel) for name in order}

    # Global scope first, then each package function OVERRIDES it — which
    # is makepkg's own rule: a `provides=` inside package_x() replaces the
    # global array for that split package.
    scope = None            # None == global
    for _n, line in lines:
        func = PKGFUNC.match(line)
        if func:
            scope = func.group(1)
            continue
        if line.startswith("}") and scope is not None:
            scope = None
            continue
        match = ASSIGN.match(line)
        if not match:
            continue
        key, raw = match.group(1), match.group(2)
        if key not in ("provides", "conflicts", "replaces"):
            continue
        vals = _values(raw, env)
        targets = [pkgs[scope]] if scope in pkgs else list(pkgs.values())
        for pkg in targets:
            pkg.kind(key)[:] = vals

    # pkgbase is NOT a shipped package when pkgname= names others, but a
    # pkgbase that equals a stock name is exactly how #284 was built — so
    # it is reported, not dropped.
    if pkgbase and pkgbase not in pkgs and not is_lisa_name(pkgbase):
        for pkg in pkgs.values():
            if pkgbase in pkg.claims():
                break
        else:
            errors.append(
                f"{rel}: `pkgbase={pkgbase}` is a foreign name and no "
                f"package here claims it with provides=/conflicts=. If the "
                f"build is a fork of `{pkgbase}`, say so by contract; if it "
                f"is not, the pkgbase is misleading."
            )
    return list(pkgs.values())


def discover_built(root, errors):
    """(name -> Pkg) for every package os/packages/** builds."""
    built, seen = {}, 0
    for pkgbuild in sorted((root / "os" / "packages").rglob("PKGBUILD")):
        seen += 1
        for pkg in read_pkgbuild(root, pkgbuild, errors):
            if pkg.name in built:
                errors.append(
                    f"{pkg.pkgbuild}: builds `{pkg.name}`, which "
                    f"{built[pkg.name].pkgbuild} also builds. Two PKGBUILDs "
                    f"for one package name means the index holds whichever "
                    f"was published last."
                )
            built[pkg.name] = pkg
    if not seen:
        errors.append(
            "no PKGBUILD found under os/packages/ at all — the discovery "
            "scan is broken, not the tree empty. A matched-nothing sweep "
            "must fail."
        )
    return built


# ----------------------------------------------------------- Packages=
#
# mkosi INI: `Packages=` opens a list, indented lines continue it, a
# `Key=` / `[Section]` / a blank-to-key transition closes it. The same
# parser reads the heredoc release.yml writes, because it IS a mkosi
# drop-in — parsing it as YAML would be parsing the wrong grammar.

KEYLINE = re.compile(r"^[A-Za-z][A-Za-z0-9]*=")
RELEASE_HEREDOC = re.compile(r"50-release\.conf\s*<<'?EOF'?")


def _packages_from_ini(lines, origin, out):
    collecting = False
    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if stripped.startswith("[") and stripped.endswith("]"):
            collecting = False
            continue
        if KEYLINE.match(stripped):
            if stripped.startswith("Packages="):
                collecting = True
                rest = stripped[len("Packages="):].strip()
                if rest:
                    out.setdefault(rest, origin)
            else:
                collecting = False
            continue
        if collecting:
            for word in stripped.split():
                out.setdefault(word, origin)


def declared_packages(root, path, errors):
    """{package name: where it was declared} for one config file."""
    full = root / path
    out = {}
    if not full.is_file():
        errors.append(
            f"{path}: a lane names this file and it does not exist, so that "
            f"lane's declared package set is empty — and an empty declared "
            f"set makes every presence check below vacuous."
        )
        return out
    text = full.read_text().splitlines()
    if path.endswith(".yml"):
        # Only the generated mkosi drop-in inside the workflow, never the
        # workflow's own prose: release.yml has a comment reading
        # "Packages=lisa-desktop-shell resolved against the …" that a
        # whole-file parse would read as a declaration.
        body, inside = [], False
        for line in text:
            if not inside:
                if RELEASE_HEREDOC.search(line):
                    inside = True
                continue
            if line.strip() == "EOF":
                break
            body.append(line)
        if not body:
            errors.append(
                f"{path}: no `50-release.conf <<EOF` heredoc found. The "
                f"release lane's Lisa packages are declared there and "
                f"nowhere else, so failing to find it would silently make "
                f"the release lane declare nothing."
            )
        text = body
    _packages_from_ini(text, path, out)
    return out


# ------------------------------------------------------------- manifest

def read_manifest(path, errors):
    """{name: version} from `pacman -Q` output.

    `-f` before `-s`, and the final line is read whether or not the file
    ends in a newline — both are check-desktop.sh's #297 lessons, and
    both were vacuous passes there.
    """
    if not path.is_file() or not path.stat().st_size:
        errors.append(
            f"{path} is not a readable non-empty file. mkosi.postinst."
            f"chroot writes it with `pacman -Q`; a build that reaches here "
            f"without one has lost that step, and a gate with nothing to "
            f"check must not pass. If you meant the built root, pass "
            f"<root>/usr/lib/lisa/packages.manifest."
        )
        return {}
    out = {}
    for n, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
        line = line.strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) < 2:
            errors.append(
                f"{path}:{n}: `{line}` is not `<name> <version>`. This file "
                f"is `pacman -Q` output; a line that is not means the write "
                f"was truncated or the file is not what the caller thinks."
            )
            continue
        out[parts[0]] = parts[1]
    if not out:
        errors.append(
            f"{path}: parsed zero package rows. Whatever this file is, it "
            f"is not a manifest, and every check below would have passed "
            f"over an empty set."
        )
    return out


# ------------------------------------------------------------- the gate

class Inventory:
    def __init__(self, root):
        self.root = root
        self.errors = []
        self.built = discover_built(root, self.errors)
        self.declared = {}          # lane -> {name: origin}
        for lane, files in LANES.items():
            merged = {}
            for path in files:
                merged.update(declared_packages(root, path, self.errors))
            self.declared[lane] = merged

        self.ours, self.forks, self.ports = {}, {}, {}
        # fork stock name -> the Lisa package that replaces it
        self.replaced = {}
        for name, pkg in sorted(self.built.items()):
            claims = pkg.claims()
            if is_lisa_name(name):
                if claims:
                    self.forks[name] = pkg
                    for stock in claims:
                        self.replaced[stock] = name
                else:
                    self.ours[name] = pkg
            elif name in PORTS:
                self.ports[name] = pkg
            else:
                self.errors.append(
                    f"{pkg.pkgbuild}: builds `{name}`, which is neither a "
                    f"`lisa-*` package nor a declared PORT. This is #284's "
                    f"shape: a PKGBUILD that TAKES a stock name and wins by "
                    f"pkgrel loses the day Arch ships a higher pkgver — it "
                    f"already did, on 2026-08-04. Rename it `lisa-*` and "
                    f"carry provides=/conflicts= on `{name}`, or — if Arch "
                    f"ships no package by that name — add it to PORTS with "
                    f"the reason."
                )
        for name, rec in OUT_OF_TREE.items():
            self.forks.setdefault(name, None)
            self.replaced.setdefault(rec["replaces"], name)

    # -- R1..R7 ------------------------------------------------------
    def check_repo(self):
        errors = self.errors

        # R2: a claim is only a contract if it is BOTH halves.
        for name, pkg in sorted(self.forks.items()):
            if pkg is None:
                continue        # out-of-tree; its PKGBUILD is another repo's
            for stock in sorted(pkg.claims()):
                has_p = any(e.split("=")[0] == stock for e in pkg.provides)
                has_c = any(e.split("=")[0] == stock for e in pkg.conflicts)
                if not has_c:
                    errors.append(
                        f"{pkg.pkgbuild}: `{name}` declares provides= on "
                        f"`{stock}` but no conflicts= on it. Both halves or "
                        f"neither: without conflicts= pacman will happily "
                        f"install stock `{stock}` ALONGSIDE the fork, and "
                        f"every path they share gets two owners — "
                        f"lisa-desktop#7, 94 paths across three pairs."
                    )
                if not has_p:
                    errors.append(
                        f"{pkg.pkgbuild}: `{name}` declares conflicts= on "
                        f"`{stock}` but no provides= on it. Then nothing "
                        f"that depends on `{stock}` — gdm, the control "
                        f"centre, half the session — can resolve at all, "
                        f"and the fork is not a replacement, it is a "
                        f"removal."
                    )

        # R3: rule 11. Nothing forks the foundation, and nothing here
        # builds a package by a foundation name.
        for stock, forker in sorted(self.replaced.items()):
            if stock in FOUNDATION:
                errors.append(
                    f"`{forker}` replaces `{stock}`, which is FOUNDATION: "
                    f"{FOUNDATION[stock]} CLAUDE.md rule 11 — toolkit and "
                    f"compositor are foundation, not experience, and rebase "
                    f"cost scales with the width of the delta. If this fork "
                    f"is really intended, it needs an ADR that moves "
                    f"`{stock}` out of FOUNDATION first."
                )
        for name in sorted(self.built):
            if name in FOUNDATION:
                errors.append(
                    f"os/packages builds a package NAMED `{name}`, which is "
                    f"FOUNDATION. Even a rebuild under the stock name is the "
                    f"pkgrel race #284 is about."
                )

        # R4: one name, one bucket.
        for name in sorted(set(FOUNDATION) & set(INTERIM)):
            errors.append(
                f"`{name}` is in both FOUNDATION and INTERIM. Those buckets "
                f"point in opposite directions — one may never be forked, "
                f"the other is waiting to be replaced."
            )
        for name in sorted((set(FOUNDATION) | set(INTERIM)) & set(PORTS)):
            errors.append(
                f"`{name}` is declared stock (FOUNDATION/INTERIM) and also a "
                f"PORT this repo builds. It cannot be both."
            )
        for stock, forker in sorted(self.replaced.items()):
            if stock in INTERIM:
                errors.append(
                    f"`{stock}` is listed INTERIM — 'stock because the Lisa "
                    f"equivalent does not exist yet' — but `{forker}` "
                    f"already replaces it. The equivalent exists; move the "
                    f"name out of INTERIM so the inventory stops claiming a "
                    f"gap that is closed."
                )

        # R5: an unclassified Lisa-adjacent DECLARED name.
        known = set(self.built) | set(OUT_OF_TREE)
        for lane, decls in sorted(self.declared.items()):
            for name, origin in sorted(decls.items()):
                if is_lisa_name(name) and name not in known:
                    errors.append(
                        f"{origin}: the `{lane}` lane declares `{name}`, "
                        f"which no PKGBUILD under os/packages builds and no "
                        f"OUT_OF_TREE record claims. Either it does not "
                        f"exist — and the lane's build dies on 'target not "
                        f"found' — or another repo ships it and nothing here "
                        f"records which. A Lisa package nobody classified is "
                        f"a Lisa package nobody checked."
                    )

        # R6: the floors. Every one of these went green over an empty set
        # in some gate audited on 2026-08-06.
        if not self.forks:
            errors.append(
                "discovery found no fork at all. lisa-desktop-shell is one "
                "by construction, so an empty set means the PKGBUILD scan "
                "or OUT_OF_TREE is broken — not that Lisa forks nothing."
            )
        if not self.ours:
            errors.append(
                "discovery found no `ours` package. os/packages/lisa alone "
                "builds eleven; an empty set is a broken scan."
            )
        for lane, decls in sorted(self.declared.items()):
            if not decls:
                errors.append(
                    f"the `{lane}` lane declares no packages at all. Its "
                    f"config files parsed to nothing, so every presence "
                    f"check for that lane would pass over an empty set."
                )

        # R7: the ratchet. Anything this file names must be somebody's
        # decision, or be named as debt under its ceiling.
        ceiling = DEBT_CEILING.get("UNDECLARED_DEBT")
        if ceiling is None:
            errors.append(
                "UNDECLARED_DEBT has no entry in DEBT_CEILING. A list that "
                "suppresses findings must have a stated maximum, or it is a "
                "convention again."
            )
        elif len(UNDECLARED_DEBT) > ceiling:
            errors.append(
                f"UNDECLARED_DEBT holds {len(UNDECLARED_DEBT)} entries and "
                f"DEBT_CEILING says at most {ceiling}: "
                f"{sorted(UNDECLARED_DEBT)}. Each entry is a load-bearing "
                f"package that reaches the image only because something "
                f"else happens to want it (#45's shape). Raise the ceiling "
                f"in the same commit and say why — that line is the one a "
                f"reviewer has to read."
            )
        elif len(UNDECLARED_DEBT) < ceiling:
            errors.append(
                f"UNDECLARED_DEBT holds {len(UNDECLARED_DEBT)} entries but "
                f"DEBT_CEILING still says {ceiling}. Lower it — a ceiling "
                f"that outlives the debt is headroom nobody decided to "
                f"grant."
            )

        anywhere = set()
        for decls in self.declared.values():
            anywhere |= set(decls)
        for name in sorted(set(FOUNDATION) | set(INTERIM)):
            if name in anywhere:
                if name in UNDECLARED_DEBT:
                    errors.append(
                        f"`{name}` IS declared in a lane's Packages= now and "
                        f"is still listed in UNDECLARED_DEBT. Delete the "
                        f"entry and lower the ceiling — the debt list exists "
                        f"to be deleted, and one that outlives its debt is "
                        f"how the next reader learns to distrust it."
                    )
            elif name not in UNDECLARED_DEBT:
                errors.append(
                    f"`{name}` is classified "
                    f"{'FOUNDATION' if name in FOUNDATION else 'INTERIM'} "
                    f"and no lane's Packages= names it, so it is on the "
                    f"device only because something else depends on it. "
                    f"That is #45's shape — libcurl arrived that way and "
                    f"`lisa update` was one upstream reshuffle from being "
                    f"unable to download. Declare it in os/mkosi/mkosi.conf, "
                    f"or add it to UNDECLARED_DEBT with the reason AND raise "
                    f"DEBT_CEILING in the same commit."
                )

    # -- M1..M7 ------------------------------------------------------
    def check_manifest(self, manifest, lane, path):
        errors = self.errors
        decls = self.declared[lane]

        # M6: does the lane the caller claims match the evidence?
        kernel = LANE_KERNEL[lane]
        if kernel not in manifest:
            errors.append(
                f"{path}: --lane {lane} was claimed, but `{kernel}` — the "
                f"one package that says which architecture built this image "
                f"— is not in the manifest. Either the lane is wrong or the "
                f"image is, and judging it under the wrong lane's declared "
                f"set would check the wrong things."
            )
        for other_kernel in sorted({k for k in LANE_KERNEL.values()
                                    if k != kernel and k in manifest}):
            owners = sorted(l for l, k in LANE_KERNEL.items()
                            if k == other_kernel)
            errors.append(
                f"{path}: --lane {lane} was claimed, but the manifest "
                f"carries `{other_kernel}`, which belongs to the "
                f"{'/'.join(owners)} lane(s)."
            )

        # M1: a stock name a fork replaces is INSTALLED.
        for stock, forker in sorted(self.replaced.items()):
            if stock not in manifest:
                continue
            if stock in decls and forker not in decls:
                # Declared stock, deliberately: the aarch64 lane and
                # gnome-shell (ADR-0021). Never silent — "the fork is not
                # here by decision" has to read differently from "the fork
                # is not here".
                print(f"inventory: STOCK BY DECLARATION — `{stock}` "
                      f"{manifest[stock]} is installed instead of `{forker}`, "
                      f"and {decls[stock]} declares it on the `{lane}` lane.")
                continue
            errors.append(
                f"{path}: `{stock}` {manifest[stock]} is INSTALLED, and "
                f"`{forker}` is the Lisa package that replaces it. No lane "
                f"config declares `{stock}`, so this is not a decision — it "
                f"is the fork losing. Two ways it happens and both are "
                f"#284: the fork took the stock NAME and lost a pkgrel race "
                f"(`vercmp 50.3-2 50.4-1` = -1; Arch shipped 50.4-1 on "
                f"2026-08-04), or the fork was never built for this lane and "
                f"pacman resolved stock to satisfy a dependency. Whichever "
                f"it is, the Lisa half of this component is not on the "
                f"device."
            )

        # M2/M3: absence fails.
        for bucket, members in (("fork", self.forks), ("ours", self.ours),
                                ("port", self.ports)):
            for name in sorted(members):
                if name not in decls or name in manifest:
                    continue
                errors.append(
                    f"{path}: `{name}` is classified {bucket} and "
                    f"{decls[name]} declares it on the `{lane}` lane, but it "
                    f"is NOT in the image. A declared package that never "
                    f"arrived is the failure this gate exists for: nothing "
                    f"else notices, because the thing that would have used "
                    f"it simply behaves as if the feature does not exist."
                )

        # M4: the foundation and the honest interim are not optional.
        for name in sorted(FOUNDATION):
            if name not in manifest:
                errors.append(
                    f"{path}: `{name}` is FOUNDATION and is not in the "
                    f"image. {FOUNDATION[name]} A desktop missing it does "
                    f"not degrade — it does not start."
                )
        for name in sorted(INTERIM):
            if name not in manifest:
                errors.append(
                    f"{path}: `{name}` is INTERIM and is not in the image. "
                    f"{INTERIM[name]} ADR-0048's interim is a promise that "
                    f"the stock app IS there until ours exists; dropping it "
                    f"leaves a hole rather than a gap."
                )

        # M5: a Lisa-namespace package nobody accounted for.
        known = set(self.built) | set(OUT_OF_TREE)
        for name in sorted(manifest):
            if is_lisa_name(name) and name not in known:
                errors.append(
                    f"{path}: `{name}` {manifest[name]} is installed and "
                    f"nothing in this repo builds it, no OUT_OF_TREE record "
                    f"claims it, and no lane declares it. A Lisa package on "
                    f"a device that no tree accounts for is either a stale "
                    f"artifact from an older index or a fourth repo nobody "
                    f"wrote down."
                )

    # -- reporting ---------------------------------------------------
    def buckets(self, manifest):
        """{bucket: [names]} over an installed set."""
        out = {b: [] for b in
               ("ours", "fork", "fork-lost", "port", "foundation", "interim",
                "transitive")}
        for name in sorted(manifest):
            if name in self.forks:
                out["fork"].append(name)
            elif name in self.replaced:
                # A STOCK name sitting in a slot a Lisa fork owns. Its own
                # bucket, not "fork": counting it as one would make the
                # table say the fork shipped.
                out["fork-lost"].append(f"{name} (want {self.replaced[name]})")
            elif name in self.ours:
                out["ours"].append(name)
            elif name in self.ports:
                out["port"].append(name)
            elif name in FOUNDATION:
                out["foundation"].append(name)
            elif name in INTERIM:
                out["interim"].append(name)
            else:
                out["transitive"].append(name)
        return out


def main(argv):
    root = Path(__file__).resolve().parents[2]
    if "--root" in argv:
        root = Path(argv[argv.index("--root") + 1]).resolve()

    inv = Inventory(root)

    if "--list" in argv:
        want = argv[argv.index("--list") + 1]
        table = {"ours": inv.ours, "fork": inv.forks, "port": inv.ports,
                 "foundation": FOUNDATION, "interim": INTERIM}
        if want not in table:
            print(f"--list takes one of {sorted(table)}", file=sys.stderr)
            return 2
        for name in sorted(table[want]):
            print(name)
        return 0

    inv.check_repo()

    manifest, lane, mpath = {}, None, None
    if "--manifest" in argv:
        if "--lane" not in argv:
            print("--manifest needs --lane " + "|".join(sorted(LANES)) +
                  ": a lane's Packages= lists are what 'declared' means, and "
                  "guessing one would decide which absences count.",
                  file=sys.stderr)
            return 2
        lane = argv[argv.index("--lane") + 1]
        if lane not in LANES:
            print(f"unknown lane `{lane}`; known: {sorted(LANES)}",
                  file=sys.stderr)
            return 2
        mpath = Path(argv[argv.index("--manifest") + 1])
        manifest = read_manifest(mpath, inv.errors)
        if manifest:
            inv.check_manifest(manifest, lane, mpath)
    elif "--lane" in argv:
        print("--lane without --manifest has nothing to judge.", file=sys.stderr)
        return 2

    if "--explain" in argv:
        print(f"{'BUCKET':12} {'COUNT':>5}  MEMBERS")
        if manifest:
            for bucket, names in inv.buckets(manifest).items():
                shown = ", ".join(names) if bucket != "transitive" else \
                    "(not policed — see the module docstring)"
                print(f"{bucket:12} {len(names):>5}  {shown}")
            print(f"{'TOTAL':12} {len(manifest):>5}  packages in {mpath}")
        else:
            for bucket, names in (("ours", sorted(inv.ours)),
                                  ("fork", sorted(inv.forks)),
                                  ("port", sorted(inv.ports)),
                                  ("foundation", sorted(FOUNDATION)),
                                  ("interim", sorted(INTERIM))):
                print(f"{bucket:12} {len(names):>5}  {', '.join(names)}")
        print()
        for stock, forker in sorted(inv.replaced.items()):
            print(f"replaces: {forker:32} -> {stock}")

    if inv.errors:
        # De-duplicated in first-seen order: three lanes share
        # os/mkosi/mkosi.conf, so one missing file would otherwise be
        # reported three times and the real findings would scroll away.
        print("\ndesktop inventory FAILED:\n", file=sys.stderr)
        seen = set()
        for e in inv.errors:
            if e in seen:
                continue
            seen.add(e)
            print(f"  {e}\n", file=sys.stderr)
        return 1

    summary = (f"desktop inventory: {len(inv.ours)} ours, {len(inv.forks)} "
               f"fork, {len(inv.ports)} port, {len(FOUNDATION)} foundation, "
               f"{len(INTERIM)} interim, {len(UNDECLARED_DEBT)} undeclared "
               f"debt")
    if manifest:
        counts = inv.buckets(manifest)
        summary += (f"; {len(manifest)} packages in {mpath}, "
                    f"{len(counts['transitive'])} transitive (unpoliced)")
    print(summary + " — OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
