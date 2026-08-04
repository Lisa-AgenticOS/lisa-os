# apps/notes — Notes (`lisa-notes`)

Spec: docs/PLAN.md §5.8. Decisions: ADR-0013 (the first real tool on the
Agent Bus), ADR-0047 (GJS + GTK4/Adwaita is the one toolkit),
**ADR-0048** (a core app — see below). Milestone: M6.

## What it does

Notes is an MCP server today, and **not yet a GUI**. `lisa-notes` listens
on `<socket_dir>/app.lisaos.notes.sock` (default `/run/lisa/mcp`), speaks
newline-delimited JSON-RPC 2.0 per `libs/mcp-bus`, and keeps notes in
SQLite under the user's XDG data dir. agentd's `McpDispatcher` connects to
it; it was the first real tool on the Agent Bus, which is why it exists
before its window does.

Tools, from `app.lisaos.notes.json`:

| tool | tier | undo |
|---|---|---|
| `create_note` | write | `delete_note` |
| `list_notes` | read | — |
| `search_notes` | read | — |
| `delete_note` | write | `restore_note` |
| `restore_note` | write | — |

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
