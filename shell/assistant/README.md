# Lisa Assistant — the chat window

Spec: docs/PLAN.md §5.7.1 · ADR: docs/adr/0015-assistant-app.md · Milestone: M4/M6.

A persistent, multi-turn chat window — the surface that makes the model
actually usable: talk to a **local** model or your **signed-in cloud** models
(Claude, GPT, …) with streaming, a model picker, and an egress marker on turns
that leave the machine. It complements the transient Super+Shift+Space overlay
(one-shot ask); it does not replace it.

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

- `lisa-assistant.js` — the window (model picker, conversation, composer).
- `lib/model.js` — pure view-model (model-list assembly, send payload, egress
  marker); unit-tested in `tests/model.test.js`.
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
