# Lisa Assistant — the chat window

Spec: docs/PLAN.md §5.7.1 · ADR: docs/adr/0015-assistant-app.md · Milestone: M4/M6.

A persistent, multi-turn chat window — the surface that makes the model
actually usable: talk to a **local** model or your **signed-in cloud** models
(Claude, GPT, …) with streaming, a model picker, and an egress marker on turns
that leave the machine. It complements the transient Super+Shift+Space overlay
(one-shot ask); it does not replace it.

While a reply streams, **Send flips to Stop** (`Overlay1.Cancel` — the partial
text stays, #11); the entry stays typeable, only sending is gated. The header
bar **exports the conversation as Markdown** (#8) — cloud turns keep their
"left this machine" note.

## Conversations

A sidebar lists every conversation, most recently active first: **New**
(header or sidebar), click to switch, trash to delete after a confirm.
Titles come from the first user turn. Each conversation **persists across
restarts** in `dev.lisaos.Context1` app memory (namespace
`app.lisaos.Assistant`) under the key layout of harness-core's `SessionStore`
(`libs/harness-core/src/session.rs`, ADR-0013) — one record per session at
`session/<id>`, one index at `sessions`, with the same field order and the
same `{role, text, model}` turns, so records written here load in Rust and
vice versa. Launch reopens the most recent conversation.

- A new conversation is written only when its **first turn completes** —
  abandoning one leaves nothing behind.
- Delete **tombstones** the record (empty string) because `Context1` has no
  per-key delete, only a namespace-wide `MemoryWipe` that would take the other
  conversations with it. Readers treat empty as absent.
- Upgrading from the single-conversation build: the old `conversation` key is
  folded into the first session on the next launch, then tombstoned so it
  happens once.
- All Context1 calls **fail soft**: without `lisa-contextd` the app runs
  exactly as before — conversations live for the run of the window, and the
  user is told once.

## How it fits

A **second thin frontend of the `dev.lisaos.Overlay1` backend** (the overlay's
"one headless backend, many frontends" design). The window sends a multi-turn
chat `Ask` and renders the streamed `Token` signals — the same contract the
GNOME Shell overlay uses.

```
lisa-assistant.js  (GJS + GTK4/Adwaita window)
  │  Overlay1.Ask(prompt, {lane:"chat", model_hint, history_json}) → id
  │  ← Token(id, delta) … Finished(id, status)
  ▼
lisa-overlayd.js  (backend chat lane)
  │  POST lisa-inferenced :7778 /v1/chat/completions (messages, stream)
  ▼
lisa-inferenced → (remote:*) → remoted broker → Claude / GPT
```

- **Models:** local from `GET /v1/models`; cloud from `dev.lisaos.Remote1`
  (providers that are signed in or hold a key → their `ListModels`). A cloud
  pick routes as `remote:<provider>:<model>`.
- **On the record:** every turn is ledgered by the daemon — `inference.*`
  for local, `remote.*` (the "leaves your hardware" marker) for cloud. This
  app renders; the daemons enforce.

## Layout

- `lisa-assistant.js` — the window (model picker, conversation list, chat log,
  composer, Stop/export, Context1 persistence).
- `lib/model.js` — pure view-model (model-list assembly, send payload, egress
  marker, Markdown export, turn (de)serialization); unit-tested in
  `tests/model.test.js`.
- `lib/sessions.js` — pure session logic (key layout, records and index,
  titles, ordering, the legacy migration); unit-tested in
  `tests/sessions.test.js`.
- `app.lisaos.Assistant.desktop` + `lisa-assistant-symbolic.svg` — launcher entry.
- The chat lane itself lives in the backend
  (`../overlay-extension/backend/lisa-overlayd.js`) with pure helpers in
  `../overlay-extension/lib/chat.js` (`tests/chat.test.js`).

## Run

```sh
gjs -m shell/assistant/lisa-assistant.js
```

Needs the per-user `lisa-inferenced` companion on `:7778` (override with
`LISA_INFERENCED_URL`). Cloud models need a provider signed in via
Settings → Intelligence and the companion's remote routing enabled
(`cfg.remote.enabled` + `LISA_REMOTED_SOCKET`, see the PKGBUILD unit).

## Tests

`just shell-test` (pure logic, any JS runtime). The window itself is verified
on the GNOME desktop (GJS is interpreted — copy and run, no image rebuild).
