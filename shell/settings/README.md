# Lisa Settings — Providers page v1

Spec: docs/PLAN.md §5.11 (BYO third-party endpoints, per-scope "may
offload" switches, distinct "leaves your hardware" color) with the §5.3
Settings-panel lineage. Decision record: ADR-0008.

The desktop management surface for the `lisa-remoted` egress broker,
over D-Bus (`org.lisa.Remote1`):

- **Remote providers:** the registry rows (OpenAI, Anthropic, Tinker,
  Together.ai, Fireworks.ai + user-added OpenAI-compat URLs), each with
  an amber *leaves your hardware* badge, write-only key entry
  (store/replace/forget — never read back), removal for custom rows,
  and a **Sign in with Claude** button on the Anthropic row that stays
  disabled *with the reason* until Anthropic publishes registerable
  OAuth endpoints (rule 8: the app never pretends).
- **What may leave this machine:** per-scope switches (`prompt`,
  `files`, `mail`, `calendar`, `screen`, `memory`), default all off; a
  banner states the measured condition ("Nothing leaves this machine."
  or the amber list of what may). Broker unreachable → defaults shown,
  switches inert.

## Layout

- `lisa-settings.js` — GTK4/libadwaita app (GJS, ESM), following the
  Ledger app's structure. All broker interaction is async D-Bus with a
  graceful offline mode.
- `lib/model.js` — pure view-model (state parsing with safe defaults,
  provider/consent rows, sign-in gating, form validation, egress color
  constants). No GTK imports.
- `tests/model.test.js` — unit tests via `shell/testing/harness.js`
  (`just shell-test`; runs under gjs, node, or macOS jsc).
- `org.lisa.Settings.desktop` — launcher entry.

## Run (dev)

`gjs -m shell/settings/lisa-settings.js` with `lisa-remoted --dbus`
running on the session bus.

## Status

First pass; packaging into the lisa-shell split package follows the
desktop lane's PKGBUILD (see PR notes). Grows with the substrate:
per-app grant management (M2 portal), context-source toggles (M3), and
the `remote:personal` node pairing UI (M7) land on adjacent pages.
