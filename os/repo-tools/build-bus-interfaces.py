#!/usr/bin/env python3
"""Generate the sdk's ONE copy of the D-Bus interfaces — and ratchet the
hand-rolled copies out of existence (ADR-0060).

The seven system interfaces were hand-declared ~60 times across the GJS
surfaces (counted 2026-08-06: Overlay1 x23, Context1 x10, Harness1 x8,
Agent1 x8, Inference1 x6, Voice1 x6, Remote1 x6). Sixty copies of an
interface is sixty places a signature change becomes a runtime error no
build caught — #218 is the recorded cost of the pattern.

Source of truth: `apps/lisa.sdk/bus/xml/*.xml`, INTROSPECTED from the
running daemons on the reference device (not copied from another hand
copy). The Rust daemons declare the interfaces with #[zbus::interface],
so live introspection is the only machine-readable form of what they
actually serve.

  (default)   regenerate apps/lisa.sdk/bus/interfaces.js from the xml/
  --check     lint gate: (a) interfaces.js in sync with xml/;
              (b) the RATCHET — a file outside apps/lisa.sdk/bus that
              declares `interface name="dev.lisaos..."` must be on the
              allowlist below, and the allowlist only ever shrinks.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
XML_DIR = ROOT / "apps/lisa.sdk/bus/xml"
OUT = ROOT / "apps/lisa.sdk/bus/interfaces.js"

# Files still carrying their own declarations, awaiting migration to
# `../lisa.sdk/bus/`. REMOVE entries as they migrate; adding one is the
# defect this gate exists to block.
ALLOWLIST = {
    "shell/assistant/lib/memory.js",
}

HEADER = """\
// GENERATED from apps/lisa.sdk/bus/xml/ — edit those (or re-introspect
// the daemons), then run python3 os/repo-tools/build-bus-interfaces.py.
// Hand edits are overwritten.
//
// The sdk's ONE copy of the system D-Bus interfaces (ADR-0060). The
// xml/ snapshots are introspected from the RUNNING daemons on the
// reference device, so this file describes what the daemons serve, not
// what a surface once believed they served.
"""


def generate() -> str:
    parts = [HEADER, "export const IFACE_XML = {"]
    for p in sorted(XML_DIR.glob("dev.lisaos.*.xml")):
        name = p.stem.rsplit(".", 1)[-1]  # Overlay1, Agent1, ...
        xml = p.read_text()
        m = re.search(r"(<node>.*</node>)", xml, re.S)
        if not m:
            sys.exit(f"{p}: no <node> element")
        body = m.group(1).replace("\\", "\\\\").replace("`", "\\`")
        parts.append(f"    {name}: `\n{body}`,")
    parts.append("};\n")
    return "\n".join(parts)


def check_ratchet() -> list[str]:
    errors = []
    # The SEVEN system interfaces, exactly: surface-private protocols
    # (e.g. dev.lisaos.Overlay1.UI, the overlay's own backend-frontend
    # edge) keep their single declaration where they live.
    pat = re.compile(
        r"interface name=[\"']dev\.lisaos\."
        r"(Overlay1|Voice1|Harness1|Agent1|Context1|Inference1|Remote1)[\"']"
    )
    import os

    for dirpath, dirnames, filenames in os.walk(ROOT):
        # Prune, don't filter: descending into node_modules/.git/target
        # and discarding afterwards made this check take a minute.
        dirnames[:] = [
            d for d in dirnames
            if d not in (".git", "target", "node_modules", ".output", ".nuxt")
        ]
        rel_dir = Path(dirpath).relative_to(ROOT).as_posix()
        if rel_dir.startswith(("apps/lisa.sdk/bus", "web")):
            dirnames[:] = []
            continue
        for f in filenames:
            if not f.endswith(".js"):
                continue
            path = Path(dirpath) / f
            rel = path.relative_to(ROOT).as_posix()
            try:
                text = path.read_text(encoding="utf-8")
            except (UnicodeDecodeError, OSError):
                continue
            if pat.search(text) and rel not in ALLOWLIST:
                errors.append(
                    f"{rel}: hand-rolled dev.lisaos interface — import "
                    f"../lisa.sdk/bus/interfaces.js instead (ADR-0060)"
                )
    for rel in sorted(ALLOWLIST):
        if not (ROOT / rel).exists():
            errors.append(f"allowlist entry {rel} no longer exists — remove it")
        elif not pat.search((ROOT / rel).read_text(encoding="utf-8")):
            errors.append(
                f"{rel} no longer declares an interface — remove it "
                f"from the allowlist (the ratchet only shrinks)"
            )
    return errors


def main() -> None:
    if "--check" in sys.argv:
        problems = []
        if not OUT.exists() or OUT.read_text() != generate():
            problems.append(
                "bus-interfaces: interfaces.js STALE — run "
                "python3 os/repo-tools/build-bus-interfaces.py"
            )
        problems += check_ratchet()
        if problems:
            print("\n".join(problems), file=sys.stderr)
            sys.exit(1)
        n = len(list(XML_DIR.glob("dev.lisaos.*.xml")))
        print(
            f"bus-interfaces: {n} interfaces in sync; "
            f"{len(ALLOWLIST)} legacy declaration site(s) remaining"
        )
        return
    OUT.write_text(generate())
    print(f"wrote {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
