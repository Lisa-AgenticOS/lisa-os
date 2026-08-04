#!/usr/bin/env python3
"""Generate the ADR index in docs/adr/README.md from the ADRs themselves.

Why this is a generator and not a written page: the hand-written version
claimed "36 of the 37 records below carry no status line" while there
were 50 records and every one of them had a status line, and its "what
is actually built" table stopped at ADR-0038 — so a reader could not
tell "not built" from "nobody looked". A page that describes 50 files
has to be derived from those 50 files or it will drift, and drift in
this page is expensive: it is the page a session reads to find out what
was already decided.

Single source of truth: each ADR's own `- **Status:**` line. The README
region between the sentinels is derived from them and is not editable
by hand.

The gate (`--check`, wired into `just lint`) fails on four things:

1. an ADR whose status line is missing or not in the canonical shape
   `- **Status:** <state>` (three different shapes were in use);
2. a status whose state is not in the vocabulary below — "accepted"
   with no execution note is allowed, but "mostly done" is not;
3. a supersession pointing at an ADR number that does not exist;
4. a README whose generated region disagrees with the ADRs.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ADR_DIR = ROOT / "docs/adr"
README = ADR_DIR / "README.md"

BEGIN = "<!-- BEGIN GENERATED INDEX — os/repo-tools/build-adr-index.py; edit the ADRs, not this table -->"
END = "<!-- END GENERATED INDEX -->"

# The controlled vocabulary. Order matters: longest match first, so
# "accepted, partially executed" is not read as "accepted".
STATES = [
    "superseded in part by",   # ...ADR-NNNN
    "superseded by",           # ...ADR-NNNN
    "accepted, partially executed",
    "accepted, not implemented",
    "accepted",
    "proposed",
    "status unverified",
]

STATUS_RE = re.compile(r"^- \*\*Status:\*\* (.+)$")
TITLE_RE = re.compile(r"^# ADR-(\d{4})\s*[—:-]\s*(.+)$")
SUPERSEDE_RE = re.compile(r"superseded (?:in part )?by ADR-(\d{4})")


def parse(path):
    """Return (number, title, state, detail) or raise ValueError."""
    lines = path.read_text().splitlines()
    if not lines:
        raise ValueError(f"{path.name}: empty file")

    m = TITLE_RE.match(lines[0])
    if not m:
        raise ValueError(
            f"{path.name}: first line must be '# ADR-NNNN — Title', got {lines[0]!r}"
        )
    number, title = m.group(1), m.group(2).strip()

    status, at = None, None
    for i, line in enumerate(lines[1:8], 1):
        m = STATUS_RE.match(line)
        if m:
            status, at = m.group(1), i
            break
    if status is None:
        raise ValueError(
            f"{path.name}: no status line in the canonical shape "
            f"'- **Status:** <state>' within the first 8 lines"
        )

    # Continuation lines are indented under the bullet; join them so the
    # status is one sentence the table can carry.
    for line in lines[at + 1:]:
        if line.startswith("  ") and line.strip():
            status += " " + line.strip()
        else:
            break
    status = re.sub(r"\s+", " ", status).strip()

    for state in STATES:
        if status == state or status.startswith(state + " "):
            break
    else:
        raise ValueError(
            f"{path.name}: status starts {status[:48]!r}; must start with one of "
            + ", ".join(repr(s) for s in STATES)
        )

    if state.startswith("superseded"):
        m = SUPERSEDE_RE.match(status)
        if not m:
            raise ValueError(
                f"{path.name}: '{state}' must name an ADR as 'superseded ... by ADR-NNNN'"
            )
        state = f"{state} ADR-{m.group(1)}"

    detail = status[len(state):].strip()
    detail = detail.lstrip("—-–;,").strip()
    return number, title, state, detail


TALLY_LABEL = {
    "superseded in part by": "superseded in part",
    "superseded by": "superseded",
    "accepted, partially executed": "accepted and partly executed",
    "accepted, not implemented": "accepted with no code yet",
    "accepted": "accepted and done",
    "proposed": "proposed",
    "status unverified": "unverified",
}


def render(records):
    counts = {}
    for _, _, state, _ in records:
        key = re.sub(r" ADR-\d{4}$", "", state)
        counts[key] = counts.get(key, 0) + 1
    tally = ", ".join(f"{counts[k]} {TALLY_LABEL[k]}" for k in STATES if k in counts)

    out = [
        BEGIN,
        "",
        f"**{len(records)} records** — {tally}.",
        "",
        "| ADR | Decision | Status | Where it actually stands |",
        "|---|---|---|---|",
    ]
    for number, title, state, detail in records:
        path = next(ADR_DIR.glob(f"{number}-*.md")).name
        cell = detail.replace("|", "\\|") or "—"
        out.append(f"| [{number}]({path}) | {title} | {state} | {cell} |")
    out += ["", END]
    return "\n".join(out)


def main():
    paths = sorted(ADR_DIR.glob("[0-9][0-9][0-9][0-9]-*.md"))
    if not paths:
        print("adr-index: no ADRs found — wrong directory?")
        return 1

    records, errors = [], []
    for path in paths:
        try:
            records.append(parse(path))
        except ValueError as exc:
            errors.append(str(exc))

    numbers = {n for n, _, _, _ in records}
    for number, _, state, _ in records:
        m = re.search(r"ADR-(\d{4})$", state)
        if m and m.group(1) not in numbers:
            errors.append(f"ADR-{number}: superseded by ADR-{m.group(1)}, which does not exist")

    if errors:
        print("adr-index: ADR status lines are not usable:")
        print("\n".join(f"  {e}" for e in errors))
        return 1

    text = README.read_text()
    if BEGIN not in text or END not in text:
        print(f"adr-index: {README} is missing the {BEGIN!r} / {END!r} sentinels")
        return 1
    head, rest = text.split(BEGIN, 1)
    _, tail = rest.split(END, 1)
    new = head + render(records) + tail

    if "--check" in sys.argv:
        if new != text:
            print(
                "adr-index: docs/adr/README.md is STALE — "
                "run python3 os/repo-tools/build-adr-index.py"
            )
            return 1
        print(f"adr-index: {len(records)} ADRs, index in sync")
        return 0

    README.write_text(new)
    print(f"adr-index: wrote docs/adr/README.md ({len(records)} ADRs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
