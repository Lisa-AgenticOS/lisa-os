<!-- GENERATED into the OS knowledge pack from apps/notes/README.md by
     os/repo-tools/build-knowledge.py — edit the source README,
     then regenerate. (#175, ADR-0040) -->

# apps/notes — Notes (`lisa-notes`)

Spec: docs/PLAN.md §5.8. Decisions: ADR-0013 (the first real tool on the
Agent Bus), ADR-0047 (GJS + GTK4/Adwaita is the one toolkit),
**ADR-0048** (a core app — see below). Milestone: M6.

## What it does

Notes is two halves that share one API.

**The daemon** (`lisa-notes`, Rust) listens on
`<socket_dir>/app.lisaos.notes.sock` (default `/run/lisa/mcp`), speaks
newline-delimited JSON-RPC 2.0 per `libs/mcp-bus`, and keeps notes in
SQLite under the user's XDG data dir. agentd's `McpDispatcher` connects
to it; it was the first real tool on the Agent Bus, which is why it
existed for months before its window did.

**The window** (`lisa-notes-app.js`, GJS) landed 2026-08-06 as the first
consumer of `apps/lisa.sdk` — ADR-0056's rule that a shared library is
extracted from a real caller rather than designed for an imagined one.
It does **not** open the SQLite store. It works through the same tool
surface the agent calls (six of the seven — the sidebar filters
client-side rather than calling `search_notes`), over the same socket,
so what the person sees and what the model sees are the same list by
construction rather than by two pieces of code agreeing. That is "apps are agent surfaces" as a fact
about the code: the tool surface is not a bolt-on beside the app, it is
the app's API, and the window is its first client.

**Status, precisely:** the window's model layer is tested (`tests/`,
cases under node), the tree is staged into the apps payload, the
`.desktop` has its PKGBUILD install line, and the window has been drawn
and used on the reference device (2026-08-06, from a scratch tree — the
released image does not carry it yet). "Works on a device" currently
means that one device, hand-deployed.

Tools, from `app.lisaos.notes.json`:

| tool | tier | undo |
|---|---|---|
| `create_note` | write | `delete_note` |
| `list_notes` | read | — |
| `read_note` | read | — |
| `update_note` | write | `update_note` with what it returned |
| `search_notes` | read | — |
| `delete_note` | write | `restore_note` |
| `restore_note` | write | — |

`update_note` is its own undo: it returns `previous_title` and
`previous_body`, and the manifest's undo block maps them straight back
into another `update_note` call. `list_notes` and `search_notes`
summaries carry a `snippet` — the first 200 characters of the body, cut
in SQL — so a sidebar can show previews without reading every note.

Deletes are **soft** in `storage.rs`, which is what makes the bus's undo
journal able to compensate rather than merely apologise.

## How it works

- `src/main.rs` — socket setup and the accept loop. `APP_ID` and the
  socket dir must match `mcp_bus::DEFAULT_SOCKET_DIR`.
- `src/server.rs` — the JSON-RPC surface and argument validation
  (length caps, required fields, typed errors).
- `src/storage.rs` — SQLite: one `notes` table with a `deleted` flag.

Rust, not GJS — it predates ADR-0047 and has no UI to speak of, which is
also why ADR-0039 §3 keeps it in `lisa-os` rather than `lisa-apps`: it is
a Cargo workspace member with a path dependency on `libs/mcp-bus`.

## Why it is a core app

ADR-0048 §5's test is *an app is core if removing it breaks a promise the
OS makes*, and Notes is the edge case that shows the test working. A note
vault sounds like the most replaceable app on the system — except
`libs/bus-tools` and `shell/assistant` hand `search_notes` to the model as
a **system capability**. Remove Notes and you have not removed an app; you
have removed a tool the assistant still advertises.

## Limits

- **No GUI.** PLAN §5.8 asks for a local vault with backlinks, embedding
  on save, "ask my notes" and native writing tools. None of that exists.
  What exists is the tool surface above. (The earlier version of this file
  described a Flutter app and said "not started" — both wrong: the lane is
  parked under ADR-0047, and the server has been running since ADR-0013.)
- **No embeddings.** `search_notes` is a case-insensitive substring match
  over title and body, not semantic search. "Every note embedded on save"
  is spec, not behaviour.
- **No backlinks, no vault format.** Notes are rows, not files.
