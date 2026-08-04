#!/usr/bin/env python3
"""Generate docs/KEYBOARD.md from the files that actually bind the keys.

Issue #257, asked for after an afternoon that produced three separate
demonstrations that a hand-written table would not survive:

  * `<Super>space` was reserved in a schema comment, GNOME's
    input-source switcher was displaced to free it, and PLAN §5.7.2
    named it — while nothing anywhere called `addKeybinding` (#255).
  * The same comment claimed the remap happened in "the image build and
    layer install", which is one file, not two.
  * Double-tap Shift is described as a summon gesture and is
    toolkit-scoped on Wayland, so it never worked system-wide (#208).

Each of those was a document describing intent as behaviour — the defect
CLAUDE.md rule 10 names. So the map is derived, never written:

  * `shell/*/schemas/*.gschema.xml` — Lisa's own chords, with the
    summary text as the description a reader wants.
  * `os/packages/lisa/10_lisa-shell.gschema.override` — the keys we set
    in GNOME's OWN schemas (Track L and the image both read this file).
  * `os/mkosi/mkosi.extra/etc/dconf/db/local.d/*` — the image's dconf
    defaults, the backend layer that survives whichever shell reads it.

The column that would have caught #255 on the day it landed is **Bound
by**: a chord with no call site is reserved, not bound, and the map says
so instead of implying otherwise.

`--check` regenerates and diffs, so `just lint` fails when a shortcut is
added, removed or rebound without the map following. Same mechanism as
`build-knowledge.py --check` and `build-adr-index.py --check`.

Parsing is imported from check-shell-keys.py rather than repeated: two
readers of one file format is how the two dock lists in #239 drifted.
"""

import argparse
import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "docs" / "KEYBOARD.md"

_spec = importlib.util.spec_from_file_location(
    "check_shell_keys", Path(__file__).with_name("check-shell-keys.py"))
_keys = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_keys)

# Groups whose keys are chords. Same predicate the checker uses, so a
# group one tool treats as bindings cannot be settings to the other.
BINDING_GROUP = _keys.BINDING_GROUP
NOT_A_CHORD = _keys.NOT_A_CHORD

# `overlay-key` is a chord by any user's reckoning — it is the bare Super
# key — but it lives in a group the binding predicate does not match and
# holds a bare key name rather than a bracketed list. Named here so the
# one key Lisa deliberately gives back does not vanish from its own map.
EXTRA_KEYS = {("org/gnome/mutter", "overlay-key")}

# Prose that names a chord we do not bind is what #255 looked like from
# the outside. Files whose whole job is to describe a chord we removed
# would trip this, so the scan is limited to the sources that a reader
# would take as current: the schemas and the two settings files.
PROSE_SOURCES = [
    "shell/*/schemas/*.gschema.xml",
    "os/packages/lisa/10_lisa-shell.gschema.override",
    "os/mkosi/mkosi.extra/etc/dconf/db/local.d/*",
]
CHORD_IN_PROSE = re.compile(
    r"\b((?:Ctrl|Control|Shift|Alt|Super)(?:\+(?:Ctrl|Control|Shift|Alt|Super))*"
    r"\+(?:[A-Za-z]|space|Space|Tab|Escape))\b")


def pretty(chord):
    """`<Shift><Super>space` → `Shift+Super+Space`, for a human."""
    mods = re.findall(r"<([^>]+)>", chord)
    rest = re.sub(r"<[^>]+>", "", chord)
    if len(rest) == 1:
        rest = rest.upper()
    else:
        rest = rest.capitalize()
    return "+".join(mods + [rest]) if rest else "+".join(mods)


def normalise(text):
    """A chord reduced to a comparable set of parts.

    Prose writes Super+Shift+Space and the schema writes
    `<Shift><Super>space`: the same chord in two orders. Comparing them
    literally is how a false "nothing binds this" gets reported, so
    modifiers are sorted and Control/Ctrl are folded together.
    """
    parts = re.split(r"[+<>]+", text.strip())
    parts = [p.lower() for p in parts if p]
    parts = ["ctrl" if p == "control" else p for p in parts]
    mods = sorted(p for p in parts if p in {"ctrl", "shift", "alt", "super"})
    rest = [p for p in parts if p not in {"ctrl", "shift", "alt", "super"}]
    return "+".join(mods + rest)


def schema_rows():
    """Lisa's own chords: schema default + summary + whether it is bound."""
    rows = []
    for schema_dir in sorted((ROOT / "shell").glob("*/schemas")):
        ext = schema_dir.parent
        source = (ext / "extension.js").read_text()
        added = _keys.string_args(source, "Main.wm.addKeybinding")
        for xml in sorted(schema_dir.glob("*.gschema.xml")):
            text = xml.read_text()
            _, _, keys = _keys.schema_keys(xml)
            summaries = dict(re.findall(
                r'<key name="([^"]+)".*?<summary>(.*?)</summary>', text, re.S))
            for key, chords in keys.items():
                bound = key in added
                for chord in chords:
                    rows.append({
                        "chord": pretty(chord),
                        "does": summaries.get(key, "").strip().replace("\n", " "),
                        "where": f"`shell/{ext.name}/schemas/{xml.name}`",
                        "bound": (f"`{ext.name}/extension.js`" if bound
                                  else "**nothing — reserved, not bound**"),
                        "scope": "Lisa",
                        "key": key,
                    })
    return rows


def settings_rows():
    """Keys we set in GNOME's own schemas — GNOME binds, we pick the chord."""
    sources = [(ROOT / "os/packages/lisa/10_lisa-shell.gschema.override",
                "the package (Track L and the image both install it)")]
    dconf = ROOT / "os/mkosi/mkosi.extra/etc/dconf/db/local.d"
    sources += [(p, "the image's dconf defaults")
                for p in sorted(dconf.glob("*")) if p.is_file()]

    rows = []
    for path, who in sources:
        for group, key, value in _keys.keyfile_groups(path.read_text()):
            is_chord = (BINDING_GROUP.search(group) and key not in NOT_A_CHORD)
            if not is_chord and (group, key) not in EXTRA_KEYS:
                continue
            chords = re.findall(r"'([^']*)'", value)
            rel = path.relative_to(ROOT)
            if not chords or chords == [""]:
                rows.append({
                    "chord": "— *(deliberately empty)*",
                    "does": f"`{key}` is unset, so GNOME's default does not fire",
                    "where": f"`{rel}` `[{group}]`",
                    "bound": f"GNOME, via {who}",
                    "scope": "GNOME key we set",
                    "key": key,
                })
                continue
            for chord in chords:
                rows.append({
                    "chord": pretty(chord),
                    "does": f"GNOME's `{key}`",
                    "where": f"`{rel}` `[{group}]`",
                    "bound": f"GNOME, via {who}",
                    "scope": "GNOME key we set",
                    "key": key,
                })
    return rows


def reserved_in_prose(known):
    """Chords named in our own settings prose that nothing in the map binds.

    This is #255 seen from the outside: the comment was right about the
    intent and wrong about the system, and no test could tell.
    """
    found = {}
    for pattern in PROSE_SOURCES:
        for path in sorted(ROOT.glob(pattern)):
            if not path.is_file():
                continue
            for line in path.read_text().splitlines():
                stripped = line.strip()
                if not (stripped.startswith("#") or stripped.startswith("<!--")
                        or "<description>" in line or "<summary>" in line
                        or stripped.startswith("-->") or stripped.startswith("*")
                        or "  " in line and not stripped.startswith("<")):
                    if not stripped.startswith("#"):
                        continue
                for chord in CHORD_IN_PROSE.findall(line):
                    if normalise(chord) in known:
                        continue
                    found.setdefault(chord, set()).add(
                        str(path.relative_to(ROOT)))
    return found


def render(rows, dangling):
    lines = [
        "# Lisa OS — the keyboard map",
        "",
        "**Generated by `os/repo-tools/build-keymap.py`. Do not edit.**",
        "Run `just lint` and it regenerates; a shortcut added, removed or",
        "rebound without this file following fails the gate.",
        "",
        "Every row is derived from a file that binds the key, never from a",
        "description of one. The **Bound by** column is the point: issue",
        "#255 was a chord reserved in a comment, freed in a settings file,",
        "named in the PLAN, and registered by nothing — which read exactly",
        "like a working shortcut until somebody pressed it.",
        "",
        "## Lisa's own chords",
        "",
        "Registered by our GNOME Shell extensions. These are **defaults, not",
        "locks**: no `locks/` directory ships, so Settings → Keyboard rebinds",
        "any of them and the change sticks.",
        "",
        "| Chord | What it does | Declared in | Bound by |",
        "|---|---|---|---|",
    ]
    for r in sorted(rows, key=lambda r: (r["scope"] != "Lisa", r["chord"])):
        if r["scope"] != "Lisa":
            continue
        lines.append(f"| `{r['chord']}` | {r['does']} | {r['where']} | {r['bound']} |")

    lines += [
        "",
        "## GNOME keys Lisa sets",
        "",
        "GNOME registers these; we choose the chord, or clear it. A row with",
        "an empty chord is a key we deliberately give back — see",
        "`20-lisa-keys` for why the bare Super key is one of them.",
        "",
        "| Chord | GNOME key | Set in | Bound by |",
        "|---|---|---|---|",
    ]
    for r in sorted(rows, key=lambda r: r["chord"]):
        if r["scope"] == "Lisa":
            continue
        chord = r["chord"] if r["chord"].startswith("—") else f"`{r['chord']}`"
        lines.append(f"| {chord} | {r['does']} | {r['where']} | {r['bound']} |")

    lines += ["", "## Chords named in prose that nothing binds", ""]
    if dangling:
        lines += [
            "Each of these appears in a comment or a description in the files",
            "above, and does not appear in either table. That is the shape of",
            "#255: intent recorded as if it were behaviour.",
            "",
            "| Chord | Named in |",
            "|---|---|",
        ]
        for chord, where in sorted(dangling.items()):
            lines.append(f"| `{chord}` | " + ", ".join(f"`{w}`" for w in sorted(where)) + " |")
    else:
        lines.append("None. Every chord our settings files talk about is in a table above.")

    lines += [
        "",
        "## What this map cannot tell you",
        "",
        "- **Whether the key press works.** Registering a binding needs the",
        "  extension loaded, which needs a session. The map proves the chord",
        "  is declared and registered, not that mutter delivered it.",
        "- **Gestures.** Double-tap Shift (#208) is toolkit-scoped on Wayland",
        "  and reaches fcitx5 only inside a GTK app, so it is not a system",
        "  shortcut and has no row here.",
        "- **What the key is called on your keyboard.** Super is ⌘ on Apple",
        "  hardware; labelling per-layout is #256, and it should read this",
        "  file rather than keep a second list.",
        "",
    ]
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true",
                    help="fail if the committed map is stale")
    args = ap.parse_args()

    rows = schema_rows() + settings_rows()
    known = {normalise(r["chord"]) for r in rows}
    text = render(rows, reserved_in_prose(known))

    if args.check:
        if not OUT.exists():
            print(f"build-keymap: {OUT.relative_to(ROOT)} does not exist; run "
                  f"os/repo-tools/build-keymap.py", file=sys.stderr)
            return 1
        if OUT.read_text() != text:
            print(f"build-keymap: {OUT.relative_to(ROOT)} is stale — a shortcut "
                  f"changed and the map did not follow. Run "
                  f"os/repo-tools/build-keymap.py", file=sys.stderr)
            return 1
        n = len(rows)
        print(f"keymap: {n} bindings, map in sync with the files that bind them")
        return 0

    OUT.write_text(text)
    print(f"wrote {OUT.relative_to(ROOT)} — {len(rows)} bindings")
    return 0


if __name__ == "__main__":
    sys.exit(main())
