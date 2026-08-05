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
            # `$comment` is prose, not a token. It is allowed inside a
            # group as well as at the top level so a group whose rules
            # are not obvious — color.account is identity, never status —
            # can carry them where someone editing it will look.
            if name.startswith("$"):
                continue
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
    # The account palette is the one group whose ORDER and MEMBERSHIP are
    # load-bearing: Mail hashes an account's Maildir root into it, so a
    # consumer needs the list, not eight lookups by name it would have to
    # keep in step by hand (#248). Flat TOKENS loses which group a colour
    # came from, and a hand-kept copy in the app is the second source of
    # truth this whole generator exists to prevent.
    accents = [
        f"    '{value}',"
        for group, _name, value, _role in flat(tokens)
        if group == "account"
    ]
    return (
        "// GENERATED from branding/tokens.json — edit that, then run\n"
        "// python3 branding/generate-tokens.py. Hand edits are overwritten.\n"
        "export const TOKENS = {\n" + "\n".join(entries) + "\n};\n"
        "// Ordered, because consumers index into it. Identity, not status.\n"
        "export const ACCOUNT_ACCENTS = [\n" + "\n".join(accents) + "\n];\n"
        f"export const FONT_UI = '{tokens['font']['ui']['value']}';\n"
    )


def theme(tokens):
    """Tailwind v4 `@theme` block — the websites' entry point.

    The two Nuxt sites used to invent their own neutrals: a cooler
    paper and ink than tokens.json sanctions, with only the violet
    matching by luck. That is the "fourth violet" defect (ADR-0038
    step 1) living in the one tree the lint gate did not cover, so
    the brand could drift on its own marketing site with nothing
    going red. Emitting a `@theme` block makes Tailwind utilities
    and the Nuxt UI theme read the same file GTK and GJS already do.

    Names follow Tailwind's `--color-<name>` convention so every
    utility (`bg-paper`, `text-ink-500`, `border-line-200`) and every
    Nuxt UI token resolves against this palette with no mapping layer
    in between — one vocabulary, four consumers.
    """
    lines = [
        "/* GENERATED from branding/tokens.json — edit that, then run",
        "   python3 branding/generate-tokens.py. Hand edits are overwritten. */",
        "",
        "@theme {",
    ]
    for _group, name, value, role in flat(tokens):
        comment = f" /* {role} */" if role else ""
        lines.append(f"  --color-{name}: {value};{comment}")
    lines.append(f"  --font-ui: \"{tokens['font']['ui']['value']}\", system-ui, sans-serif;")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


def main():
    tokens = json.loads((ROOT / "tokens.json").read_text())
    out = ROOT / "out"
    want = {
        out / "tokens.css": css(tokens),
        out / "tokens.js": js(tokens),
        out / "tokens.theme.css": theme(tokens),
    }

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
