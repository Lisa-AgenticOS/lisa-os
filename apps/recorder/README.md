# apps/recorder — Recorder

Spec: docs/PLAN.md §5.8. Decisions: ADR-0047 (GJS + GTK4/Adwaita is the
one toolkit), **ADR-0048** (a store app, not a core app). Milestone: M6.

## Status: not started

**Nothing here but this file.** No code, no capture pipeline, no agent
surface.

## What it is meant to be

Live transcription, diarization-lite, and a meeting summary with action
items offered to the Agent Bus ("add these 3 todos?"). GJS + GTK4/Adwaita
like every other Lisa app since ADR-0047 — the earlier plan named a
Flutter lane that was never built and is now parked.

## Where it sits: the store side, deliberately

ADR-0048 §5 draws the core/store line with a test — *an app is core if
removing it breaks a promise the OS makes* — and Recorder is the edge case
that makes the test worth having.

It **feels** like a system utility. It is not. Nothing on the system
depends on it: no agent tool disappears when it is absent (unlike
`apps/notes`, whose `search_notes` the assistant advertises), and it is
not the default handler for anything the desktop must handle. So it is
independently installable, with its own version and channel, rather than
part of the desktop payload.

That mechanism does not exist yet — the app channel is monolithic today
(one `shell` tarball, one version) and per-app payloads follow issue #239.

## Before writing code here

Read PLAN §5.7.5 (voice: ASR via whisper.cpp, the model lineup in §7) and
§5.8. Then read `apps/preview` for the house pattern — pure logic in
`lib/`, `lib/mcp-protocol.js` for the JSON-RPC surface, `lib/mcp.js` for
the socket, tests through `shell/testing/harness.js`.

The constraint that shapes this app: **a transcript is untrusted text.**
Anything said in a meeting reaches the model as `provenance` that is not
`user`, and a summary that proposes tool calls must escalate a
confirmation tier, not act.

## Limits

Everything. There is no app, no measured ASR throughput on the reference
iMac, and no diarization approach chosen — PLAN's "diarization-lite" is a
wish, not a design.
