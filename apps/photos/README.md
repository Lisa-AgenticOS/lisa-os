# apps/photos — Lisa Photos

Spec: docs/PLAN.md §5.8. Decisions: **ADR-0048** (Lisa Desktop is a
desktop, not a patched GNOME), ADR-0047 (GJS + GTK4/Adwaita is the one
toolkit). Milestone: M6.

## Status: not started

**Nothing here but this file.** No code, no window, no agent surface, no
captioning pipeline. This directory records a decision, not an
implementation.

Until it exists, image *viewing* on the reference device is served by
`apps/preview`, which opens images and PDFs and exposes what is open to
the assistant. Preview is a viewer, not a library — it has no import, no
album, no search.

## What it is meant to be

A first-party photo library in the shape ADR-0047 settled — GJS, GTK4 and
libadwaita, app id `app.lisaos.Photos` — MCP-native from the first commit.

The capabilities PLAN §5.8 asks for:

- local VLM captioning and tagging on import, at background QoS so it
  never competes with a foreground request
- natural-language search over those captions ("the whiteboard from
  March")

## Why it is an app and not a GNOME Photos patch set

This directory was `apps/photos-patches` until 2026-08-04 and contained
exactly one file: a README saying "not started". Zero patches were ever
written. ADR-0048 carries the argument.

The Photos case is the sharpest of the three, because the feature *is*
the integration: captioning on import means an import pipeline that calls
`inferenced` with a VLM at background QoS and writes results somewhere the
context index can search. That is not a hook you add to somebody else's
importer — it is the importer.

## Before writing code here

Read PLAN §5.8 (Photos), §5.9 (the acceleration matrix — captioning is
where a background VLM either fits on the reference hardware or does not)
and §7 (the model lineup, which is where the VLM row comes from). Then
read `apps/preview` for the house pattern and `apps/mail` for how a large
list is paged: Mail's list froze on a 3,758-message inbox until paging was
added, and a photo library is the same problem with bigger items.

Two constraints:

- **Captions are model output about untrusted input.** A caption derived
  from an image is `provenance: "file"`, not `"user"`, and a picture of
  text is a prompt-injection vector.
- **Background QoS is a promise to the battery and to the foreground
  session.** An import that saturates the GPU while somebody is waiting on
  the assistant is a defect, not a tuning issue.

## Limits

Everything. There is no app. Nothing here has been prototyped, and no VLM
has been measured for on-import captioning throughput on the reference
iMac — until one has, PLAN §5.8's captioning story is intent rather than
behaviour.
