<!-- GENERATED into the OS knowledge pack from shell/assistant/README.md by
     os/repo-tools/build-knowledge.py — edit the source README,
     then regenerate. (#175, ADR-0040) -->

# Lisa Assistant — the chat window

Spec: docs/PLAN.md §5.7.1 · ADR: docs/adr/0015-assistant-app.md · Milestone: M4/M6.

A persistent, multi-turn chat window — the surface that makes the model
actually usable: talk to a **local** model or your **signed-in cloud** models
(Claude, GPT, …) with streaming, a model picker, and an egress marker on turns
that leave the machine. It complements the transient Super+Shift+Space overlay
(one-shot ask); it does not replace it.

While a reply streams, **Send flips to Stop** (`Harness1.Cancel`, #11); the
entry stays typeable, only sending is gated. Stop ends the run between turns
and mid-answer — the words that already arrived stay, marked `⚠ Stopped.` —
and it un-sticks the composer. If the harness daemon leaves the bus mid-run
the window says so and ends the run rather than sitting on "Stop" for ever
(#227). The header bar **exports the conversation as Markdown** (#8) — cloud
turns keep their "left this machine" note, and a save that cannot happen says
why rather than reporting itself as a dismissal (#234).

## Attachments (#209)

The composer has a **paperclip** (a `Gtk.FileDialog` filtered to png, jpg,
jpeg, webp, gif) and takes **Ctrl+V** of an image from the clipboard. Staged
images appear as removable chips above the entry; the sent turn shows the
thumbnail above its text, so the transcript keeps the half of the question
that was a picture.

On send they become OpenAI content parts on the user turn:

```
{"type":"image_url","image_url":{"url":"data:image/png;base64,…"}}
```

passed to `Harness1.Run` as an `attachments` option — a JSON string holding
that array, the same way `history` travels. The daemon puts the message
**text first**, then the parts; with no attachments the turn stays a plain
string on the wire, unchanged. The bytes ride inside the request as a data
URI, so nothing is uploaded and there is no temporary object with a URL to
leak. This is the shape `lisa ask --attach` already builds.

A **local model plus an attachment is refused in the window**, naming the
model and saying to pick a cloud one. `lisa-inferenced`'s llama backend
already refuses content parts — a text model handed an image would otherwise
answer confidently about a picture nobody looked at — but that refusal is
five layers away, after a spinner. `attachmentRefusal` in
`lib/attachments.js` is the same rule applied where a person can still act
on it: a courtesy, not the guard. An unknown model fails closed.

**Size is bounded at attach time** (#226): one image up to 8 MiB, and up to
16 MiB across a whole send. A picture over that is refused by the chip that
would have held it, naming the file, its size and the ceiling — before a
round trip, which is where the old answer (`413`) arrived. The composer's
ceiling is the smallest of a chain: 16 MiB of bytes is 21.4 MiB of base64,
under harnessd's 24 MiB `attachments` cap, inside inferenced's 32 MiB request
limit. Those last two are compile-time assertions in their own crates, not
comments. This one is a courtesy — `parse_attachments` in harnessd is the
bound that holds for every caller on the bus.

**A location with no local path is refused, not ignored** (#234). GIO returns
a null path for a Drive mount, an `sftp://` share, a camera — the paperclip,
the working-folder chooser and export all treated that as "nothing to do", so
attaching from Drive did nothing at all and choosing a non-local folder
silently revoked the working-folder grant. `lib/chooser.js` gives the three
outcomes three names.

Limits, because they are not obvious:

- **A working folder cannot be given back.** The folder button re-picks;
  nothing revokes. The docstring used to claim "picking nothing clears the
  grant, so there is always a way to take it back" and that path was
  unreachable — dismissing the dialog throws and returns. The one thing that
  did clear a grant was choosing a non-local folder, by accident, silently,
  which is #234. Closing the window drops it. A deliberate revoke is not
  built, so this says so rather than describing one.
- **Images only.** `lisa ask --attach` also takes wav/mp3; nothing in this
  window picks or records audio, so the composer does not offer it.
- **Thumbnails do not survive a restart.** The stored session shape is
  `{role, text, model}` byte for byte — what harness-core's `SessionStore`
  reads — and the image bytes are not in it. A reopened conversation shows
  the text of a turn that had a picture, without the picture.
- **No resizing.** A 12 MP screenshot is sent at 12 MP.

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
- A read that **failed** is not a namespace that is **empty** (#228). The
  listing is loaded with one `MemoryList`, which answers `{}` for an
  empty namespace and an error for an unreadable one; the `sessions`
  index — the one write here that REPLACES rather than adds — is only
  ever written once such a read has said what it is replacing. Until
  then a completed turn still writes its own `session/<id>` record,
  because that is additive, and a record with no index entry is
  recoverable where a lost record is not.
- On launch the listing is **reconciled against the records
  themselves** (`indexFromRecords`), so conversations that dropped off
  the index come back. They needed to: `_memoryGet` used to map every
  failure — `AccessDenied` included — to `''`, which parses as an empty
  index, and the next completed turn wrote an index of exactly one
  entry over the real one. The other records were never deleted, and
  that is what makes recovery possible at all.
- A **staged image belongs to the conversation it was staged in**
  (#235). Switching conversations clears the strip and says so; the
  attachment also carries its session id, so a switch path that forgot
  to clear still cannot put one conversation's picture on another's
  wire — which matters because the other conversation may be on a cloud
  model when this one was local.
- A **Spotlight hand-off that arrives mid-stream is queued**, not
  dropped (#233). It starts its own conversation when the running reply
  finishes, and says so while it waits. It never writes into the
  composer: that draft belongs to whoever typed it.

## How it fits

A thin frontend of the **`dev.lisaos.Harness1`** loop in `lisa-harnessd`
(ADR-0025) — the overlay's "one headless backend, many frontends" design, in
the vocabulary the overlay already speaks (`Token`/`Finished`). The overlay
keeps its own one-shot chat lane; real work happens here, because that lane
skips the Agent Bus and left the assistant with no tools at all.

```
lisa-assistant.js  (GJS + GTK4/Adwaita window)
  │  Harness1.Run(prompt, {model, trigger, history, workspace?,
  │                        attachments?}) → run_id
  │  ← Tool(id, name, detail) … Token(id, delta) … Finished(id, ok, summary)
  ▼
lisa-harnessd  (the agent loop, Agent Bus + workspace + skills tools)
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
- `lib/attachments.js` — pure attachment logic (image mime by extension, the
  `image_url` content part, the parts payload, the local-model refusal, the
  size budget); unit-tested in `tests/attachments.test.js`.
- `lib/chooser.js` — what a `Gtk.FileDialog` callback actually returned:
  dismissed, a local path, or a location with no local path (#234);
  unit-tested in `tests/chooser.test.js`.
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

## Limits

Written down because the alternative is a reader inferring them from
silence — and because rule 10 asks every component to say what it does
*not* do.

- **Read-tier tools only.** The Assistant runs on `dev.lisaos.Harness1`,
  which reaches the Agent Bus through `bus-tools`, so it can search your
  mail and notes and read a page — and each call is narrated as its own
  line (`_onTool`), because what the model DID and what it SAID should
  not read the same. It cannot send, file or delete: write-tier parks
  for confirmation, and the consent surface that answers those became a
  separate process only recently (#145). Write tier is now defensible
  and not yet wired.
- **No memory across conversations.** Sessions persist (the same layout
  harness-core's `SessionStore` uses), so a conversation survives a
  restart — but nothing is recalled *between* them. harness-core's
  `Memory` exists and this window does not use it.
- **Markdown renders; the model's other output shapes do not.** Tables
  become their raw pipes, and images are not fetched — Pango markup has
  no block model, which `lib/markdown.js` says more about.
- **No voice.** PLAN §5.7.5 is not built here; the overlay's
  push-to-talk is a separate surface.
- **One persona.** harness-core calls this pillar "Soul" and rates it
  partial: the persona is a caller-supplied string, with no profiles,
  tiers, or delegation.

The honest summary: this is a chat window that can *look things up* and
cannot yet *act*. The remaining gap is write tier, and it was gated on
the consent surface becoming a separate process (#145, closed) — which
is the thing that had to land first, and now has.

An earlier draft of this section said the window had no Agent Bus client
at all. That was wrong: it was written from grepping this file for
`Agent1`, which finds only tooltips, without following
`_harness.RunSync` to `dev.lisaos.Harness1` and from there to
`bus-tools`. Checking the running daemon is what corrected it.
