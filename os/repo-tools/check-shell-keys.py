#!/usr/bin/env python3
"""A keybinding Lisa declares must be bound, compiled, and unclaimed.

Issue #255: `<Super>space` was reserved in a comment, GNOME's
input-source switcher was displaced to Ctrl+Super+Space to keep it free,
PLAN §5.7.2 named it as the Spotlight key — and nothing anywhere called
`Main.wm.addKeybinding`. The launcher had no `schemas/` directory at
all. Preparation without connection, the same shape as #241 (a manifest
installed where nothing reads it) and #244 (a .service nobody called a
method on).

Comments are not mechanisms (#192, #159), so this asserts the mechanism:

  1. Every key in a shell extension's gschema is passed to BOTH
     `Main.wm.addKeybinding` and `Main.wm.removeKeybinding` in that
     extension's extension.js. A schema key nobody adds is a chord that
     does nothing; one nobody removes leaks the binding when the
     extension is disabled, and the next enable throws.
  2. An extension with a schemas/ directory declares `settings-schema`
     in metadata.json matching the schema id, and the PKGBUILD compiles
     that directory. `getSettings()` does not degrade when the schema is
     missing or unnamed — it throws, and the extension fails to enable.
  3. No two things Lisa ships claim the same chord: extension schema
     defaults, the package's gschema override, and the image's dconf
     defaults are all read together. Two claims on one chord is
     whichever mutter happened to resolve first, which is not a
     decision anybody made. This is what freed `<Super>space` from the
     `toggle-overview` stand-in.
  4. `switch-input-source` is remapped by us, explicitly. The overlay's
     schema header has always claimed the image and layer move GNOME's
     input-source switcher off Super+Space. GNOME's own default really
     is `['<Super>space','XF86Keyboard']`, so if we ever stop setting
     it, the claim becomes a description of an accident.

Run by `just lint`; costs milliseconds and no shell session.
"""

import json
import re
import sys
from pathlib import Path
from xml.etree import ElementTree

ROOT = Path(__file__).resolve().parents[2]
SHELL = ROOT / "shell"
PKGBUILD = ROOT / "os" / "packages" / "lisa" / "PKGBUILD"
OVERRIDE = ROOT / "os" / "packages" / "lisa" / "10_lisa-shell.gschema.override"
DCONF_DIR = ROOT / "os" / "mkosi" / "mkosi.extra" / "etc" / "dconf" / "db" / "local.d"

# Groups in the override / dconf files whose keys are keybindings. A
# group not listed here is settings, not chords.
BINDING_GROUP = re.compile(r"keybindings|mutter")
# Keys in those groups that are not chords.
NOT_A_CHORD = {"workspaces-only-on-primary", "dynamic-workspaces", "edge-tiling",
               "attach-modal-dialogs", "focus-change-on-pointer-rest"}


def string_args(text, call):
    """Names passed to `call(...)`, resolving `const X = 'y'` indirection.

    The overlay passes literals; the launcher passes a shared constant so
    add and remove cannot drift. Both are the same assertion.
    """
    consts = dict(re.findall(r"const\s+([A-Za-z_$][\w$]*)\s*=\s*'([^']*)'", text))
    names = set()
    for arg in re.findall(re.escape(call) + r"\(\s*([^,\s)]+)", text):
        arg = arg.strip()
        if arg.startswith("'") and arg.endswith("'"):
            names.add(arg[1:-1])
        elif arg in consts:
            names.add(consts[arg])
    return names


def schema_keys(path):
    """{key: [chords]} and the schema id/path, from the gschema XML."""
    root = ElementTree.parse(path).getroot()
    schema = root.find("schema")
    keys = {}
    for key in schema.findall("key"):
        default = (key.find("default").text or "").strip()
        keys[key.get("name")] = re.findall(r"'([^']*)'", default)
    return schema.get("id"), schema.get("path"), keys


def pkgbuild_compiles(ext_name, pkgbuild):
    """Does the package actually run glib-compile-schemas over this dir?

    Either a loop that both iterates `$share/*/schemas` AND compiles the
    variable it binds — naming the glob while compiling something else is
    the failure this exists to catch — or the directory spelled out.
    """
    loop = re.search(
        r'for\s+(\w+)\s+in\s+"\$share"/\*/schemas;.*?glib-compile-schemas\s+"\$\1"',
        pkgbuild, re.S)
    return bool(loop) or f'glib-compile-schemas "$share/{ext_name}/schemas"' in pkgbuild


def keyfile_groups(text):
    """[(group, key, value)] for .override and dconf keyfiles alike."""
    out = []
    group = None
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            group = line[1:-1]
            continue
        m = re.match(r"([A-Za-z0-9-]+)\s*=\s*(.*)", line)
        if m and group is not None:
            out.append((group, m.group(1), m.group(2).strip()))
    return out


def main():
    problems = []

    def fail(msg):
        problems.append(msg)

    pkgbuild = PKGBUILD.read_text()
    # Every chord anybody ships → the list of things claiming it.
    claims = {}

    def claim(chords, who):
        for chord in chords:
            if chord:
                claims.setdefault(chord, []).append(who)

    # ---- 1 & 2: extension schemas are wired and compiled -------------
    for schema_dir in sorted(SHELL.glob("*/schemas")):
        ext = schema_dir.parent
        for xml in sorted(schema_dir.glob("*.gschema.xml")):
            schema_id, schema_path, keys = schema_keys(xml)
            rel = xml.relative_to(ROOT)
            meta = json.loads((ext / "metadata.json").read_text())
            if meta.get("settings-schema") != schema_id:
                fail(f"{ext.name}/metadata.json declares settings-schema "
                     f"{meta.get('settings-schema')!r}, but {rel} defines "
                     f"{schema_id!r}. getSettings() throws on the mismatch and "
                     f"the extension never enables.")
            expected_path = f"/{schema_id.replace('.', '/')}/"
            if schema_path != expected_path:
                fail(f"{rel} has path {schema_path!r}; getSettings() expects "
                     f"{expected_path!r}.")

            source = (ext / "extension.js").read_text()
            added = string_args(source, "Main.wm.addKeybinding")
            removed = string_args(source, "Main.wm.removeKeybinding")
            for key, chords in keys.items():
                if key not in added:
                    fail(f"{rel} declares {key!r} and {ext.name}/extension.js "
                         f"never calls Main.wm.addKeybinding for it — the chord "
                         f"{chords} is reserved and dead (#255).")
                if key not in removed:
                    fail(f"{ext.name}/extension.js adds {key!r} but never removes "
                         f"it in disable(); the binding outlives the extension.")
                claim(chords, f"{ext.name} schema key {key}")

        if not pkgbuild_compiles(ext.name, pkgbuild):
            fail(f"the PKGBUILD compiles no schemas for {ext.name}: an uncompiled "
                 f"schema makes getSettings() throw at enable time.")

    # ---- 3: nothing claims a chord twice ------------------------------
    for source in [OVERRIDE] + sorted(p for p in DCONF_DIR.glob("*") if p.is_file()):
        for group, key, value in keyfile_groups(source.read_text()):
            if not BINDING_GROUP.search(group) or key in NOT_A_CHORD:
                continue
            claim(re.findall(r"'([^']*)'", value),
                  f"{source.name} [{group}] {key}")

    for chord, who in sorted(claims.items()):
        if len(who) > 1:
            fail(f"{chord} is claimed by {len(who)} bindings — "
                 + ", ".join(who) + ". Which one mutter resolves is not a "
                 "decision anybody made.")

    # ---- 4: the input-source remap is ours, not inherited -------------
    remap = [(g, v) for g, k, v in keyfile_groups(OVERRIDE.read_text())
             if k == "switch-input-source"]
    if not remap:
        fail("nothing sets switch-input-source. GNOME's default is "
             "['<Super>space','XF86Keyboard'], so the Spotlight chord would be "
             "shared with the input-source switcher and the schema comments "
             "that say otherwise would be describing an accident.")
    for _, value in remap:
        if "<Super>space" in re.findall(r"'([^']*)'", value):
            fail("switch-input-source still claims <Super>space, which the "
                 "launcher binds.")

    if problems:
        for p in problems:
            print(f"check-shell-keys: {p}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
