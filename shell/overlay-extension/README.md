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
  `org.lisa.Overlay1` on the session bus: `Ask(prompt, options) →
  query_id`, `Cancel`, `GetStatus`; signals `Started(id, meta_json)`,
  `Token(id, text)`, `Finished(id, status, detail)`. Per Ask it runs
  [my stuff] retrieval via `lisa context search` (ledgered by the CLI),
  fences hits with provenance per Appendix C, opens an
  `org.lisa.Inference1` session, and re-emits the token fd as signals.
  `backend/org.lisa.Overlay1.service` provides D-Bus activation.
- `extension.js` + `metadata.json` + `schemas/` + `stylesheet.css` —
  the GNOME Shell frontend (ESM, GNOME 46+): keybinding, chips, entry,
  streamed response, footer showing attached context and ledgering.
  Also owns **`org.lisa.Overlay1.UI`** on the session bus
  (`Summon(prompt, options)`, `Hide`, `GetVisible`) — the UI-control
  surface other shell surfaces use to summon the overlay
  programmatically; the §5.7.2 launcher's "Ask Lisa" lane hands its
  queries over here. Owned by the frontend because the headless
  backend has no UI; the wlr client can own the same name.
- `lib/` — shared pure logic (`envelope.js`: Appendix C fencing, CLI
  output parsing; `iface.js`: the D-Bus interface XML).
- `tests/` — unit tests for `lib/` (`just shell-test`; runs under gjs,
  node, or macOS jsc).

## Status

Working first pass: backend + GNOME frontend wired end-to-end against
`org.lisa.Inference1` (needs a Linux/GNOME session to run; logic is
unit-tested everywhere). [this window] waits on §5.7.4 screen context
(M6); [selection] waits on §5.7.3 layer 3; both are reported
`unavailable` in Started meta. M5 swaps the backend's direct inference
call for Agent Bus (MCP) planning without changing the D-Bus surface.

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
