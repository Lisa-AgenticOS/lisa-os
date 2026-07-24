# Ledger app — the transparency centerpiece

Spec: docs/PLAN.md §5.7.6. Milestone: M4.

Renders the append-only audit DB: every model call, context retrieval,
and (M5) tool execution — filterable, tap-through to the prompt
envelope. Usage stats and export included. If a Golden Gate user asks
"what did Siri actually read?" there is no answer; this app is ours.

## Layout

- `lisa-ledger-app.js` — GTK4/libadwaita app (GJS, ESM). Timeline of
  ledger events with completions folded into their start entries;
  header dropdowns filter by app / kind / day; activating a row opens
  the envelope detail (input blake3, bounded preview, detail, tokens,
  duration, linked ledger ids); footer shows tokens-by-app; export
  writes the filtered view as JSON. Reads via `lisa ledger --json`
  (CLAUDE.md rule 7 — the CLI is the command center; the app renders
  and never writes, which the DB's append-only triggers enforce
  anyway).
- `app.lisaos.LedgerApp.desktop` — launcher entry.
- `lib/model.js` — pure view-model (timeline fold, filters, stats).
- `tests/model.test.js` — unit tests (`just shell-test`).

## Status

Working first pass; runs anywhere gjs + GTK4/libadwaita exist (needs a
populated ledger, i.e. a host where lisa-inferenced has run). Grows
with the substrate: the full prompt envelope (context chunks +
provenance) appears here when the M2 portal work attaches it; grant
management shortcuts land with the portal Settings panel; tool
executions land with M5.

Run (dev): `gjs -m shell/ledger-app/lisa-ledger-app.js` with the
`lisa` CLI on PATH (or `LISA_CLI=…`).

Install (packaged): ships in the `lisa-shell` package
(os/packages/lisa) — tree under `/usr/share/lisa/shell/`, desktop
entry in `/usr/share/applications/`. The Track I release image folds
it in.
