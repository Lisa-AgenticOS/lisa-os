# Assistant overlay

Spec: docs/PLAN.md §5.7.1. Milestone: M4.

Super+Shift+Space translucent layer with per-invocation context toggles:
[this window], [selection], [my stuff]. (Super+Space alone opens the
Spotlight-style search — shell/launcher, §5.7.2.) One headless D-Bus
backend, thin frontends: GNOME Shell extension here; the
wlr-layer-shell client (Omarchy/Hyprland, Track L) consumes the same
backend interface.

## Layout

- `backend/lisa-overlayd.js` — the headless backend (GJS). Owns
  `dev.lisaos.Overlay1` on the session bus: `Ask(prompt, options) →
  query_id`, `Cancel`, `Respond(query_id, approve)`, `GetStatus`;
  signals `Started(id, meta_json)`, `Token(id, text)`,
  `ConfirmationNeeded(id, spec_json)`, `Finished(id, status, detail)`.
  Per Ask it first tries the **Agent Bus lane** (M5, ADR-0013):
  `dev.lisaos.Agent1.Discover(prompt)` scored by `lib/agent.js` (no
  model in this lane); a confident, arg-fillable hit becomes
  `RequestCall` with actor `overlay`, provenance `["user"]`. Results
  and denial/failure reasons stream back as `Token` + `Finished`;
  parked calls raise `ConfirmationNeeded` and wait for `Respond`.
  Confirmations parked by *other* clients (`lisa do` without a TTY
  answer) surface too, via Agent1's `ConfirmationRequested` signal
  (own calls are filtered by actor — the signal precedes the
  `RequestCall` reply, so id-matching would race). Prompts that don't
  route keep the inference lane unchanged: [my stuff] retrieval via
  `dev.lisaos.Context1.Search` (lisa-contextd ledgers the retrieval
  before replying, PLAN §5.3), falling back to the `lisa context
  search` shell-out (ledgered by the CLI) when the context daemon
  isn't on the bus, then Appendix C fencing,
  `dev.lisaos.Inference1` session, token fd re-emitted as signals.
  `backend/dev.lisaos.Overlay1.service` provides D-Bus activation.
- `extension.js` + `metadata.json` + `schemas/` + `stylesheet.css` —
  the GNOME Shell frontend (ESM, GNOME 46+): keybinding, chips, entry,
  streamed response, footer showing attached context and ledgering.
  The Agent Bus lane renders as a consent surface: chip-weight box for
  `confirm-chip`, heavier modal-weight box for `confirm-modal`
  (escalated chains, destructive tiers, and non-undoable calls call
  out their warnings), Allow/Deny answering via `Respond`; one consent
  at a time, further requests queue. Also owns
  **`dev.lisaos.Overlay1.UI`** on the session bus
  (`Summon(prompt, options)`, `Hide`, `GetVisible`) — the UI-control
  surface other shell surfaces use to summon the overlay
  programmatically; the §5.7.2 launcher's "Ask Lisa" lane hands its
  queries over here. Owned by the frontend because the headless
  backend has no UI; the wlr client can own the same name.
- `lib/` — shared pure logic (`envelope.js`: Appendix C fencing,
  Context1 JSON + CLI output parsing; `agent.js`: prompt→tool routing, schema-driven arg
  filling, outcome formatting, consent-spec mapping; `iface.js`: the
  D-Bus interface XML).
- `tests/` — unit tests for `lib/` (`just shell-test`; runs under gjs,
  node, or macOS jsc).

- `backend/voice-service.js` — push-to-talk capture (§5.7.5). Owns
  `dev.lisaos.Voice1` on the **same process** as Overlay1 (a second bus
  name, and its own activation file — activation is per-name):
  `StartListening() → session_id`, `StopListening(id)`, `Cancel(id)`,
  `GetState`; signals `ListeningStarted`, `Transcribing`,
  `Transcribed(id, text)`, `Failed(id, reason)`. It spawns the recorder
  (`pw-record`/`parecord`/`arecord`, 16 kHz mono) and shells out to
  `lisa transcribe`, so the whisper model is resolved in exactly one
  place. **It is in the backend and not the extension because an
  extension runs inside the compositor**: waiting on whisper there would
  freeze every window on the machine while Lisa thought.
  `lib/voice.js` holds the decidable parts — recorder argv, transcript
  cleaning, and the one-at-a-time state machine — and is unit-tested.

  The audio is deleted as soon as it is transcribed, and `Transcribed`
  carries text rather than a path, so no surface is ever handed a
  recording it could keep. A recording is capped at 120 s: a key-release
  lost to a compositor restart would otherwise leave the microphone open
  until the session ended, with nothing reporting it.

  **Nothing is captured unless the key is held.** There is no wake word,
  no timer and no always-on capture; the microphone opens on
  `StartListening` and closes on `StopListening`, both driven by a key a
  person is physically holding, and an indicator is on screen for
  exactly that long. An ambient loop is a different design with a
  different consent story (ADR-0011) and needs its own ADR first.

## Status

Working first pass: backend + GNOME frontend wired end-to-end against
`dev.lisaos.Inference1` (needs a Linux/GNOME session to run; logic is
unit-tested everywhere). The Agent Bus lane routes actionable prompts
to `dev.lisaos.Agent1` (read-tier calls with the trusted `["user"]`
chain execute silently and render their result; write/destructive park
for chip/modal consent per the tier table). [this window] waits on
§5.7.4 screen context (M6); [selection] waits on §5.7.3 layer 3; both
are reported `unavailable` in Started meta.

**Push-to-talk (§5.7.5) is wired but has not run on hardware.** The
logic is unit-tested and the JS parses, but no one has held the key on
the reference iMac — the two engines it needs (`whisper.cpp`, `piper`)
were only just packaged and reach a device with the next release. Until
then this is code that should work, which is not the same claim as code
that does.

Known gaps (Agent1 surface, reported — not worked around): no signal
when a pending confirmation is answered elsewhere or expires, so a
consent box for another client's call can linger until clicked (the
stale `Confirm` then errors and the box closes honestly); `Discover`
omits scores, so the overlay re-implements agentd's token-overlap
ranking client-side to threshold it (kept in sync with
`daemons/agentd/src/registry.rs` by hand); arg filling is a local
heuristic — calls that need the intent-router model to split an
utterance across several arguments stay on the inference lane.

Install (dev): symlink this directory into
`~/.local/share/gnome-shell/extensions/lisa-overlay@lisa-os.org`, run
`glib-compile-schemas schemas/`, install the service file, re-log.
GNOME's input-source switcher also claims Super+Space; the image/layer
remaps it to Ctrl+Super+Space (see `schemas/` and
os/packages/lisa/10_lisa-shell.gschema.override).

Install (packaged): the `lisa-shell` package (os/packages/lisa) ships
this tree under `/usr/share/lisa/shell/`, the extension as a symlink in
`/usr/share/gnome-shell/extensions/`, the D-Bus activation file, and a
gschema override that default-enables the extension and moves the
input-source switcher to Super+Shift+Space. The Track I release image
folds it in.
