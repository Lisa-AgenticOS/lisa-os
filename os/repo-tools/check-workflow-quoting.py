#!/usr/bin/env python3
"""Syntax-check every `run:` block in the GitHub workflows.

The release workflow runs its whole container build as one single-quoted
argument to `bash -ec '...'`. A single quote anywhere inside that block —
including in a COMMENT — closes the string early, and the shell then
parses the rest as code and fails somewhere unrelated:

    # is handled in the PKGBUILD's prepare(): upstream's
                                ^ closes the string
    line 104: syntax error near unexpected token `('

An English possessive in a comment is the likely way this comes back,
which is exactly why a reviewer will not catch it, and why it cost a
full release build to find.

Rather than guess at shell quoting — `'\\''` is a legitimate way to write
an apostrophe, so "contains a quote" is not the test — hand each block
to `bash -n` and let bash decide. That is the same parser that will run
it, so there are no false positives to argue with.
"""
import pathlib
import re
import subprocess
import sys

WORKFLOWS = pathlib.Path(".github/workflows")
# `${{ ... }}` is GitHub's, not bash's; it is substituted before the
# shell ever sees it. Replace with a plain word so bash can parse.
EXPR = re.compile(r"\$\{\{.*?\}\}", re.S)


def run_blocks(path):
    """Yield (first_line_number, dedented_script) for each `run: |`."""
    lines = path.read_text().split("\n")
    i = 0
    while i < len(lines):
        m = re.match(r"^(\s*)run: \|-?\s*$", lines[i])
        if not m:
            i += 1
            continue
        indent = len(m.group(1))
        start = i + 1
        body = []
        j = start
        while j < len(lines):
            line = lines[j]
            if line.strip() and (len(line) - len(line.lstrip())) <= indent:
                break
            body.append(line[indent + 2:] if len(line) > indent else "")
            j += 1
        yield start + 1, "\n".join(body)
        i = j


def main():
    if not WORKFLOWS.is_dir():
        print("no .github/workflows — nothing to check")
        return 0

    failures = 0
    checked = 0
    for path in sorted(WORKFLOWS.glob("*.yml")):
        for line_no, script in run_blocks(path):
            checked += 1
            proc = subprocess.run(
                ["bash", "-n"],
                input=EXPR.sub("GHEXPR", script),
                capture_output=True, text=True)
            if proc.returncode != 0:
                failures += 1
                detail = proc.stderr.strip().replace("\n", "\n    ")
                print(f"{path}: the `run:` block starting at line {line_no} "
                      f"is not valid bash:\n    {detail}", file=sys.stderr)

    if failures:
        print("\nA stray apostrophe inside a `bash -ec '...'` argument is the "
              "usual cause —\nwrite \"prepare() in the PKGBUILD\" rather than "
              "\"the PKGBUILD's prepare()\".", file=sys.stderr)
        return 1
    print(f"workflow shell syntax: ok ({checked} run blocks)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
