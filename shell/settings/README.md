# Lisa Settings — AI / Intelligence panel

Spec: docs/PLAN.md §5.3 (Settings panel), §5.11 (BYO third-party
endpoints, per-scope "may offload" switches, distinct "leaves your
hardware" color), §8 (hardware-aware local-model fit). Decision record:
ADR-0008.

The desktop AI hub, a two-section libadwaita window switched from the
header (and a bottom bar when narrow):

- **Local models** — the §8 catalog from `lisa models catalog --json`,
  ordered by what runs on THIS machine: installed first, then models that
  run, then tight, then remote-only. Each row carries a plain-words fit
  badge (*installed* / *runs on this Mac* / *tight fit* / *too big — use
  a provider*) and a one-click **Get** for pinned models that actually
  fit. Local inference never leaves the machine, so nothing here is
  egress-marked. If the CLI/daemon can't answer, the page says so instead
  of guessing capacity.
- **Providers** — the `lisa-remoted` broker over D-Bus
  (`org.lisa.Remote1`): one card per provider (registry rows: OpenAI,
  Anthropic, Tinker, Together.ai, Fireworks.ai, Hugging Face + user-added
  OpenAI-compat URLs) with its **brand logo** (bundled
  `assets/provider-logos/`, recolored to the theme; a generic mark for
  custom endpoints and Tinker), an amber *leaves your hardware* badge, a
  *key set / no key* pill, write-only key entry (store/replace/forget —
  never read back), removal for custom rows, and a **Sign in with
  Claude** button on the Anthropic card that stays disabled *with the
  reason* until Anthropic publishes registerable OAuth endpoints (rule 8:
  the app never pretends). Once a key is set **and** the `prompt` scope
  is on, a **Models** row fetches the provider's own live `/models` list
  over D-Bus (`ListModels`, with a spinner and inline errors + retry) and
  offers the real ids in a dropdown — picking one copies its ready-to-use
  route `remote:<id>:<model>` to the clipboard and shows it. Without a
  key or consent, the card shows the routing guidance
  (`remote:<id>:<model-id>` + the provider's notes) instead — never an
  invented model list. Below the cards, **What may leave this machine**:
  per-scope switches (`prompt`, `files`, `mail`, `calendar`, `screen`,
  `memory`), default all off; a banner states the measured condition.
  `prompt` is first and marked *required for remote* — inferenced always
  sends it, so the broker refuses every remote request while it is off: a
  keyed provider with `prompt` off raises a prominent amber warning at
  the top of the page, and key + `prompt` on shows a subtle *Ready* note.
  Broker unreachable → defaults shown, switches and the add/save actions
  disabled with the reason.

## Layout

- `lisa-settings.js` — GTK4/libadwaita app (GJS, ESM). Providers/consent
  are async D-Bus with a graceful offline mode; local models come from
  the `lisa` CLI via `Gio.Subprocess` (one source of truth for §8 fit
  logic — Rust, not reimplemented in JS).
- `lib/model.js` — pure view-model: broker-state parsing with safe
  defaults, provider/consent rows, sign-in gating, form validation,
  remote-readiness (`remoteReadiness`) and offline
  (`providersDisabledReason`) helpers, **plus** catalog parsing,
  local-model rows + fit badges, profile summary, per-provider routing
  help, the logo map (`providerLogoFile`), the route builder
  (`modelHintFor`), and the `ListModels` reply parser (`parseModelList`).
  No GTK imports.
- `assets/provider-logos/` — the bundled brand SVGs (construct's set,
  mapped to Lisa's provider ids) plus `generic.svg`; all draw in
  `currentColor`, recolored to the theme at load.
- `tests/model.test.js` — unit tests via `shell/testing/harness.js`
  (`just shell-test`; runs under gjs, node, or macOS jsc).
- `org.lisa.Settings.desktop` — launcher entry.

## Run (dev)

`gjs -m shell/settings/lisa-settings.js`. Providers need `lisa-remoted
--dbus` on the session bus; Local models need the `lisa` CLI on PATH
(the `--json` model output landed alongside this panel — an older `lisa`
shows the "could not read the local catalog" fallback).

## Status

Two sections live. Grows with the substrate: per-app grant management
(M2 portal), context-source toggles (M3), a real default-model / voice
section once those settings are daemon-readable, and the
`remote:personal` node pairing UI (M7) land on adjacent pages.
