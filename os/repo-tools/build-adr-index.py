#!/usr/bin/env python3
"""Generate the ADR index in docs/adr/README.md from the ADRs themselves,
and check every claim an ADR makes about what exists.

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

## Claims — the part that makes a status falsifiable

A status line is prose, and prose cannot go red. Until 2026-08-06
nothing in this repo could tell a true status from a false one, and both
directions had already cost something: ADR-0050 said no `lisa dev check`
existed on a day it was the Forge's default verifier (a status that
understates invites deleting working code to match it), and ADR-0036
said "nothing is implemented" while its §6 shell tool shipped with all
four of its conditions.

So an ADR may — and, if its status asserts implementation, MUST —
carry a `- **Claims:**` block naming artifacts that must be true for the
status to be true:

    - **Claims:**
      - `path:libs/forge-harness/src/shell_tool.rs` — §6's shell tool
      - `symbol:fn contain@libs/forge-harness/src/jail.rs` — §6.1
      - `absent:libs/lisa_ui` — the directory is still free

Four kinds — two pairs, because a status can be wrong in two
directions and only one of them was ever being caught:

* `path:<repo-relative path>` — the file or directory must exist.
* `symbol:<regex>@<repo-relative path>` — the file must exist AND
  contain a match. This is how "the verb exists", "the test exists" and
  "the unit ships" are said: point at the clap arm, the `fn`, the
  `PKGBUILD` line.
* `absent:<repo-relative path>` — the path must NOT exist.
* `nomatch:<regex>@<repo-relative path>` — the file must exist and must
  NOT contain a match.

The second of each pair is what catches a status that *understates* —
the kind ADR-0050 was, and the kind that gets working code deleted to
match a document. "`modeld` is not on zbus" and "`libs/lisa_ui` does not
exist yet" are claims; the only thing that falsifies them is the thing
appearing, so they are checked by their appearance.

**What a `symbol:` claim proves and what it does not.** It is a static
read of the source: it proves the tree says so, not that the binary
runs. Running verbs would need a build, and a gate that skips itself
when the binary is missing is a gate that is green on an empty checkout
— which is the failure mode every gate audited on 2026-08-06 had.

## What the gate (`--check`, wired into `just lint`) fails on

1. an ADR whose status line is missing or not in the canonical shape
   `- **Status:** <state>` (three different shapes were in use);
2. a status whose state is not in the vocabulary below — "accepted"
   with no execution note is allowed, but "mostly done" is not;
3. a supersession pointing at an ADR number that does not exist;
4. a README whose generated region disagrees with the ADRs;
5. **a claim that is not true**: a `path:`/`symbol:`/`nomatch:` target
   that is missing, a `symbol:` whose pattern does not match, an
   `absent:` target that has appeared, a `nomatch:` whose pattern has;
6. **a missing claim**: a status asserting implementation (`accepted`,
   `accepted, partially executed`) with no positive claim behind it;
7. **a shrinking corpus**: fewer claims, or fewer ADRs carrying them,
   than the ratchet below. Removing a claim is allowed; removing it
   *quietly* is not — it costs an edit to a named constant here and a
   regenerated table in the README, both of which are visible in a diff.

Rules 5 and 6 are the ones that fail on ABSENCE rather than on
disagreement, which is the whole point: a gate that only fires when two
things disagree is green the day one of them disappears.
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

# The states that assert something exists on a machine right now, and so
# owe at least one positive claim. "accepted, not implemented" and
# "proposed" owe nothing — they assert the opposite. A superseded record
# is history and is not held to the floor either; the ADR that replaced
# it carries the present tense.
ASSERTS_IMPLEMENTATION = {"accepted", "accepted, partially executed"}

# The ratchet. Both numbers are floors, both are edited by hand, and
# lowering either is a deliberate line in a diff — which is the only
# thing standing between "we removed a stale claim" and "the corpus
# quietly emptied out". Raise them when you add claims.
MIN_CLAIMS = 142
MIN_CLAIMED_ADRS = 53

STATUS_RE = re.compile(r"^- \*\*Status:\*\* (.+)$")
TITLE_RE = re.compile(r"^# ADR-(\d{4})\s*[—:-]\s*(.+)$")
SUPERSEDE_RE = re.compile(r"superseded (?:in part )?by ADR-(\d{4})")
CLAIMS_HEAD_RE = re.compile(r"^- \*\*Claims:\*\*\s*$")
CLAIM_ITEM_RE = re.compile(r"^\s+- `([^`]+)`(?:\s*[—-]\s*(.*))?$")

KINDS = ("path", "symbol", "absent", "nomatch")
POSITIVE_KINDS = ("path", "symbol")


class ClaimError(ValueError):
    """A claim that cannot be evaluated. Never silently skipped."""


def parse_claims(path, lines):
    """Return [(kind, arg, note)] for the file's `- **Claims:**` block.

    A block with no items is an error rather than an empty list: an
    empty claim block reads like diligence and checks nothing.
    """
    head = None
    for i, line in enumerate(lines):
        if CLAIMS_HEAD_RE.match(line):
            if head is not None:
                raise ClaimError(f"{path.name}: two `- **Claims:**` blocks")
            head = i
    if head is None:
        return []

    claims = []
    for line in lines[head + 1:]:
        if not line.strip():
            break
        m = CLAIM_ITEM_RE.match(line)
        if not m:
            if line.startswith(" "):
                raise ClaimError(
                    f"{path.name}: claim line is not '  - `kind:arg` — note': {line!r}"
                )
            break
        body, note = m.group(1), (m.group(2) or "").strip()
        kind, _, arg = body.partition(":")
        if kind not in KINDS:
            raise ClaimError(
                f"{path.name}: claim kind {kind!r} is not one of "
                + ", ".join(KINDS)
            )
        if not arg:
            raise ClaimError(f"{path.name}: claim {body!r} has no argument")
        claims.append((kind, arg, note))

    if not claims:
        raise ClaimError(
            f"{path.name}: `- **Claims:**` block with no claims under it"
        )
    return claims


def claim_target(kind, arg):
    """The repo-relative path a claim points at, plus the pattern if any."""
    if kind in ("symbol", "nomatch"):
        pattern, sep, rel = arg.rpartition("@")
        if not sep:
            raise ClaimError(f"symbol claim {arg!r} must be '<regex>@<path>'")
        if len(pattern) < 3 or set(pattern) <= set(".*+?^$"):
            raise ClaimError(
                f"symbol claim {arg!r}: pattern {pattern!r} is too weak to fail"
            )
        try:
            rx = re.compile(pattern, re.M)
        except re.error as exc:
            raise ClaimError(f"symbol claim {arg!r}: bad regex ({exc})") from exc
    else:
        pattern, rx, rel = None, None, arg

    rel = rel.strip()
    if rel.startswith("/") or ".." in Path(rel).parts:
        raise ClaimError(f"claim path {rel!r} must be repo-relative and inside the repo")
    if rel.split("/")[0] == "docs":
        # A claim about the record is not a claim about the system:
        # `path:docs/PLAN.md` is true on every day of this project's
        # life and therefore says nothing.
        raise ClaimError(f"claim path {rel!r} is under docs/ — claim the artifact, not the prose")
    return rx, rel


def evaluate(kind, arg):
    """Return None if the claim holds, else why it does not."""
    rx, rel = claim_target(kind, arg)
    target = ROOT / rel

    if kind == "path":
        return None if target.exists() else f"path does not exist: {rel}"

    if kind == "absent":
        return None if not target.exists() else f"path exists but was claimed absent: {rel}"

    # symbol / nomatch. A missing file fails BOTH: "modeld does not
    # declare zbus" is not a thing a deleted modeld can be said to
    # satisfy, and a claim that passes because its subject vanished is
    # the vacuous-by-absence gate this whole mechanism exists to avoid.
    if not target.exists():
        return f"file does not exist: {rel}"
    if target.is_dir():
        return f"{kind} claim points at a directory: {rel}"
    text = target.read_text(errors="replace")
    hit = rx.search(text)
    if kind == "symbol":
        return None if hit else f"pattern {rx.pattern!r} not found in {rel}"
    return None if not hit else f"pattern {rx.pattern!r} found in {rel}, but was claimed absent"


def parse(path):
    """Return (number, title, state, detail, claims) or raise ValueError."""
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

    base_state = state
    if state.startswith("superseded"):
        m = SUPERSEDE_RE.match(status)
        if not m:
            raise ValueError(
                f"{path.name}: '{state}' must name an ADR as 'superseded ... by ADR-NNNN'"
            )
        state = f"{state} ADR-{m.group(1)}"

    detail = status[len(base_state):].strip()
    detail = detail.lstrip("—-–;,").strip()

    claims = parse_claims(path, lines)
    if base_state in ASSERTS_IMPLEMENTATION:
        if not any(k in POSITIVE_KINDS for k, _, _ in claims):
            raise ValueError(
                f"{path.name}: status {base_state!r} asserts something exists, so it "
                "needs at least one `path:` or `symbol:` claim under a "
                "`- **Claims:**` block. An `absent:` claim does not count — it "
                "asserts the opposite."
            )
    return number, title, state, detail, claims


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
    for _, _, state, _, _ in records:
        key = re.sub(r" ADR-\d{4}$", "", state)
        counts[key] = counts.get(key, 0) + 1
    tally = ", ".join(f"{counts[k]} {TALLY_LABEL[k]}" for k in STATES if k in counts)
    claims = sum(len(c) for _, _, _, _, c in records)
    claimed = sum(1 for _, _, _, _, c in records if c)

    out = [
        BEGIN,
        "",
        f"**{len(records)} records** — {tally}.",
        "",
        f"**{claims} machine-checked claims across {claimed} records.** The "
        "`Checks` column is how many artifacts each ADR names that must exist "
        "(or must still be absent) for its status to be true; "
        "`build-adr-index.py --check` verifies every one of them, and a status "
        "that asserts implementation without any is a red build.",
        "",
        "| ADR | Decision | Status | Checks | Where it actually stands |",
        "|---|---|---|---|---|",
    ]
    for number, title, state, detail, claim_list in records:
        path = next(ADR_DIR.glob(f"{number}-*.md")).name
        cell = detail.replace("|", "\\|") or "—"
        n = len(claim_list) or "—"
        out.append(f"| [{number}]({path}) | {title} | {state} | {n} | {cell} |")
    out += ["", END]
    return "\n".join(out)


def main():
    paths = sorted(ADR_DIR.glob("[0-9][0-9][0-9][0-9]-*.md"))
    if not paths:
        print("adr-index: no ADRs found — wrong directory?")
        return 1

    explain = "--explain" in sys.argv
    records, errors = [], []
    for path in paths:
        try:
            records.append(parse(path))
        except ValueError as exc:
            errors.append(str(exc))

    numbers = {n for n, _, _, _, _ in records}
    for number, _, state, _, _ in records:
        m = re.search(r"ADR-(\d{4})$", state)
        if m and m.group(1) not in numbers:
            errors.append(f"ADR-{number}: superseded by ADR-{m.group(1)}, which does not exist")

    total_claims = 0
    for number, _, _, _, claim_list in records:
        for kind, arg, note in claim_list:
            total_claims += 1
            try:
                why = evaluate(kind, arg)
            except ClaimError as exc:
                why = str(exc)
            if why:
                errors.append(f"ADR-{number}: claim `{kind}:{arg}` FAILED — {why}")
            elif explain:
                print(f"  ok   ADR-{number}  {kind}:{arg}" + (f"  ({note})" if note else ""))

    claimed_adrs = sum(1 for _, _, _, _, c in records if c)
    if total_claims < MIN_CLAIMS:
        errors.append(
            f"claim corpus shrank: {total_claims} claims, floor is {MIN_CLAIMS}. "
            "Lower MIN_CLAIMS in this file if that is deliberate — it should be "
            "a line somebody has to defend in review."
        )
    if claimed_adrs < MIN_CLAIMED_ADRS:
        errors.append(
            f"claim corpus shrank: {claimed_adrs} ADRs carry claims, floor is "
            f"{MIN_CLAIMED_ADRS}. Lower MIN_CLAIMED_ADRS in this file if that is "
            "deliberate."
        )

    if errors:
        print("adr-index: ADR records are not usable:")
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
        print(
            f"adr-index: {len(records)} ADRs, index in sync, "
            f"{total_claims} claims across {claimed_adrs} records all true"
        )
        return 0

    README.write_text(new)
    print(
        f"adr-index: wrote docs/adr/README.md ({len(records)} ADRs, "
        f"{total_claims} claims across {claimed_adrs} records)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
