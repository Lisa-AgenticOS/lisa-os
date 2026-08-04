# apps/files — Lisa Files

Spec: docs/PLAN.md §5.8. Decisions: **ADR-0048** (Lisa Desktop is a
desktop, not a patched GNOME), ADR-0047 (GJS + GTK4/Adwaita is the one
toolkit). Milestone: M6.

## Status: not started

**Nothing here but this file.** No code, no window, no agent surface.
This directory records a decision, not an implementation, and it will say
so until someone writes the app.

Until then the image ships **stock GNOME Files, unpatched**. That is the
expected interim state, not a gap to be closed with a patch set.

## What it is meant to be

A first-party file manager in the shape ADR-0047 settled — GJS, GTK4 and
libadwaita, app id `app.lisaos.Files` — and MCP-native from the first
commit rather than retrofitted, the way `apps/mail`, `apps/preview` and
`apps/surfer` are built.

The capabilities PLAN §5.8 asks for:

- a semantic search bar, over the contextd index rather than filename
  matching
- "ask this folder" — scoped RAG against a directory the user points at
- auto-suggested organisation, with batch moves behind a confirm tier

## Why it is an app and not a Nautilus patch set

This directory was `apps/files-patches` until 2026-08-04 and contained
exactly one file: a README saying "not started". Zero patches were ever
written, which is what made changing course nearly free. ADR-0048 carries
the argument; the short version is three points:

- The agent surface is not a patch. Every result must carry provenance,
  the tool set has to be declared in a manifest the bus can enforce
  (ADR-0046 §2), and nothing about that is a small diff against a C file
  manager whose maintainers agreed to none of it.
- The delta only grows. `os/packages/gnome-control-center-lisa` is three
  lines of registration plus two subtractive edits and already needs four
  `grep` tripwires to survive a GNOME bump.
- Interpreted source is load-bearing (ADR-0047, ADR-0046 Amendment 1): a
  fix reaches the reference device by `scp`, and the artifact a reviewer
  reads is the artifact that runs.

## Before writing code here

Read PLAN §5.8, ADR-0048, and ADR-0047. Then read `apps/preview` — the
smallest complete example of the house pattern: pure logic in `lib/`,
`lib/mcp-protocol.js` for the JSON-RPC surface, `lib/mcp.js` for the
socket at `$XDG_RUNTIME_DIR/lisa/mcp/<app>.sock`, and unit tests through
`shell/testing/harness.js` (`just shell-test`).

Two constraints that are easy to get wrong late:

- **A file manager reads untrusted bytes.** Filenames, extended
  attributes and document contents are all injection surfaces. Results
  handed to the agent are `provenance: "file"`, never `"user"` — the same
  rule Preview follows, for the same reason.
- **Destructive operations are the whole risk.** Move, rename, trash and
  batch anything sit at write or destructive tier. Tier is enforced by
  `read_tier_tools` in `libs/bus-tools` and by the guard catalogue in
  `cli/lisa/src/guard.rs`, not by the manifest asking nicely.

## Limits

Everything. There is no app. The file *chooser* is GTK's and stays GTK's
— this directory is about the file manager, and ADR-0048's reversal
conditions name "Lisa Files turns out to be a multi-year project" as
genuine evidence against the plan.
