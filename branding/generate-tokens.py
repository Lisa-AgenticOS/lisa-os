#!/usr/bin/env python3
"""Generate the consumable token sheets from branding/tokens.json.

Outputs (committed, so packages ship them without running Python):
  branding/out/tokens.css — GTK @define-color names + CSS custom
      properties, loadable by every GJS app's CssProvider and by the
      Shell theme.
  branding/out/tokens.js  — an ES module for GJS code that needs a
      color as a *value* (Clutter.Color, drawing, badge logic).

`--check` regenerates into memory and fails if the committed outputs
drifted — the lint gate runs it, so tokens.json and its outputs cannot
disagree on main.

Why generated files are committed rather than built: the PKGBUILDs
copy file trees; a build step that exists only to rename JSON keys is
a build dependency nobody needs. The check mode is what keeps the
copies honest (the same trade README-vs-spec makes everywhere else in
this repo).
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
HEADER = "/* GENERATED from branding/tokens.json — edit that, then run\n   python3 branding/generate-tokens.py. Hand edits here are overwritten. */\n"


def flat(tokens):
    """(group, name, value, role) for every color token."""
    for group, entries in tokens["color"].items():
        for name, spec in entries.items():
            yield group, name, spec["value"], spec.get("role", "")


def css(tokens):
    lines = [HEADER]
    # GTK named colors: the form GJS apps already use (@define-color
    # lisa_violet ...) — one name per token, lisa_ prefixed, kebab→snake.
    for _group, name, value, role in flat(tokens):
        comment = f" /* {role} */" if role else ""
        lines.append(f"@define-color lisa_{name.replace('-', '_')} {value};{comment}")
    lines.append("")
    # The same palette as custom properties, for WebKit-rendered surfaces
    # (Surfer pages, Mail's HTML view chrome) where @define-color means
    # nothing.
    lines.append(":root {")
    for _group, name, value, _role in flat(tokens):
        lines.append(f"  --lisa-{name}: {value};")
    lines.append(f"  --lisa-font-ui: \"{tokens['font']['ui']['value']}\";")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


def js(tokens):
    entries = [
        f"    '{name}': '{value}',"
        for _group, name, value, _role in flat(tokens)
    ]
    return (
        "// GENERATED from branding/tokens.json — edit that, then run\n"
        "// python3 branding/generate-tokens.py. Hand edits are overwritten.\n"
        "export const TOKENS = {\n" + "\n".join(entries) + "\n};\n"
        f"export const FONT_UI = '{tokens['font']['ui']['value']}';\n"
    )


def main():
    tokens = json.loads((ROOT / "tokens.json").read_text())
    out = ROOT / "out"
    want = {out / "tokens.css": css(tokens), out / "tokens.js": js(tokens)}

    if "--check" in sys.argv:
        stale = [
            str(p) for p, text in want.items()
            if not p.exists() or p.read_text() != text
        ]
        if stale:
            print(f"tokens: STALE outputs {stale} — run python3 branding/generate-tokens.py")
            return 1
        print("tokens: outputs match tokens.json")
        return 0

    out.mkdir(exist_ok=True)
    for p, text in want.items():
        p.write_text(text)
        print(f"wrote {p}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
