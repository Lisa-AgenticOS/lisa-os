#!/usr/bin/env python3
"""Every GTK4 `set_parent()` needs a matching `unparent()` in the same file.

Issue #258. `apps/surfer/lisa-surfer.js` attached the address-bar
suggestion popover with `set_parent(urlBar)` and never unparented it, so
GTK finalized the entry with a child still on it:

    Finalizing GtkEntry 0x5575131873b0, but it still has children left:
      - GtkPopover 0x557513261910

Two Surfer sessions on the reference device ended exactly there — no
coredump, no signal, no JS error. The process shut down, which from the
user's side is a window vanishing mid-click. Whether that caused the
report is unproven and stays unproven; unclean teardown is a defect on
its own merits, and it is the kind that behaves differently each run,
which fits a failure that stopped reproducing.

The issue's acceptance said "asserted by review of each call site".
Review is how the second call site was found, not a thing that keeps
finding it — `apps/preview/lisa-preview.js` had the discipline and Surfer
did not, and nothing compared them. So this asserts it instead.

Deliberately coarse: same file, not same code path. A checker that tried
to prove the unparent is reachable from every disposal would need to
understand GJS control flow, and would be wrong in both directions. This
catches the case that actually happened — a `set_parent` with no
`unparent` anywhere near it — and stays quiet otherwise.

Run by `just lint`.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SURFACES = ["apps/*/*.js", "apps/*/lib/*.js", "shell/*/*.js", "shell/*/lib/*.js"]

SET_PARENT = re.compile(r"\.set_parent\s*\(")
UNPARENT = re.compile(r"\.unparent\s*\(")


def main():
    problems = []
    checked = 0
    for pattern in SURFACES:
        for path in sorted(ROOT.glob(pattern)):
            if not path.is_file() or "/tests/" in str(path):
                continue
            text = path.read_text()
            attaches = SET_PARENT.findall(text)
            if not attaches:
                continue
            checked += len(attaches)
            if not UNPARENT.search(text):
                rel = path.relative_to(ROOT)
                line = next(i for i, l in enumerate(text.splitlines(), 1)
                            if SET_PARENT.search(l))
                problems.append(
                    f"{rel}:{line} calls set_parent() and the file never calls "
                    f"unparent(). GTK4 finalizes the parent with the child still "
                    f"attached — the warning that preceded two Surfer sessions "
                    f"ending with no crash and no window (#258).")

    if problems:
        for p in problems:
            print(f"check-widget-lifecycle: {p}", file=sys.stderr)
        return 1
    print(f"widget-lifecycle: {checked} set_parent call(s), each with an unparent")
    return 0


if __name__ == "__main__":
    sys.exit(main())
