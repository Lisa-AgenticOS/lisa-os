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
    """Yield (first_line_number, script) for every `run:` entry.

    The first version matched exactly `run: |` — one style, one space.
    Single-line entries, folded scalars (`run: >`), `|+`, and a second
    space all skipped silently, and every miss landed inside the same
    "ok (N run blocks)" success line. About twenty single-line entries
    were shipping unchecked. So: every scalar style is handled, and the
    caller asserts the count.
    """
    lines = path.read_text().split("\n")
    i = 0
    while i < len(lines):
        m = re.match(r"^(\s*)(?:-\s+)?run:\s*(.*?)\s*$", lines[i])
        if not m:
            i += 1
            continue
        indent, rest = len(m.group(1)), m.group(2)
        block = re.match(r"^([|>])[+-]?\s*(?:#.*)?$", rest)
        if not block:
            # Single-line scalar. Unwrap the two YAML quoting styles
            # (their escapes differ); a plain scalar passes through.
            script = rest
            if len(script) >= 2 and script[0] == script[-1] == "'":
                script = script[1:-1].replace("''", "'")
            elif len(script) >= 2 and script[0] == script[-1] == '"':
                script = script[1:-1].replace('\\"', '"').replace("\\\\", "\\")
            if script:
                yield i + 1, script
            i += 1
            continue
        style = block.group(1)
        start = i + 1
        body, j, content_indent = [], start, None
        while j < len(lines):
            line = lines[j]
            if line.strip():
                this_indent = len(line) - len(line.lstrip())
                if this_indent <= indent:
                    break
                if content_indent is None:
                    # YAML fixes the block's indent from its first
                    # non-empty line — hardcoding indent+2 mis-sliced
                    # any block indented deeper.
                    content_indent = this_indent
                body.append(line[content_indent:])
            else:
                body.append("")
            j += 1
        if style == ">":
            # Folding joins the lines with spaces (blank line = real
            # newline) — hand bash what the shell will actually get.
            script = "\n".join(
                " ".join(p.split("\n")) for p in "\n".join(body).split("\n\n")
            )
        else:
            script = "\n".join(body)
        yield start + 1, script
        i = j


def main():
    if not WORKFLOWS.is_dir():
        print("no .github/workflows — nothing to check")
        return 0

    failures = 0
    checked = 0
    for path in sorted(list(WORKFLOWS.glob("*.yml")) + list(WORKFLOWS.glob("*.yaml"))):
        for line_no, script in run_blocks(path):
            checked += 1
            # A run block containing any ${{ }} is templated as a WHOLE,
            # and GitHub caps templated strings at 21000 characters — the
            # dispatch then fails to parse ("Exceeded max expression
            # length"), which is how every release dispatch silently
            # broke between 2026-08-04 evening and 2026-08-05 (fixed in
            # 564c9f0 by passing VER via env:). Gate at 20000 so the
            # failure arrives with margin, at lint time, not at the next
            # release. Blocks with no expression have no cap.
            if "${{" in script and len(script) > 20000:
                failures += 1
                print(f"{path}: the `run:` block starting at line {line_no} "
                      f"contains a ${{{{ }}}} expression and is {len(script)} "
                      f"characters — GitHub templates such a block whole and "
                      f"rejects it past 21000. Pass the value through `env:` "
                      f"so the block carries no expression.", file=sys.stderr)
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
    if checked == 0:
        # Workflows exist but nothing matched: the extractor is broken,
        # and "ok (0 run blocks)" would be the silent no-op this script
        # exists to prevent.
        print("workflow shell syntax: matched no run entries at all — "
              "the extractor is broken, not the workflows clean", file=sys.stderr)
        return 1
    print(f"workflow shell syntax: ok ({checked} run entries)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
