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
  `lisa context search` (ledgered by the CLI), Appendix C fencing,
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
- `lib/` — shared pure logic (`envelope.js`: Appendix C fencing, CLI
  output parsing; `agent.js`: prompt→tool routing, schema-driven arg
  filling, outcome formatting, consent-spec mapping; `iface.js`: the
  D-Bus interface XML).
- `tests/` — unit tests for `lib/` (`just shell-test`; runs under gjs,
  node, or macOS jsc).

## Status

Working first pass: backend + GNOME frontend wired end-to-end against
`dev.lisaos.Inference1` (needs a Linux/GNOME session to run; logic is
unit-tested everywhere). The Agent Bus lane routes actionable prompts
to `dev.lisaos.Agent1` (read-tier calls with the trusted `["user"]`
chain execute silently and render their result; write/destructive park
for chip/modal consent per the tier table). [this window] waits on
§5.7.4 screen context (M6); [selection] waits on §5.7.3 layer 3; both
are reported `unavailable` in Started meta.

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
