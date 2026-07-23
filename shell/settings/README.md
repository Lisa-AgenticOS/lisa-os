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
  (`org.lisa.Remote1`): registry rows (OpenAI, Anthropic, Tinker,
  Together.ai, Fireworks.ai, Hugging Face + user-added OpenAI-compat
  URLs), each with an amber *leaves your hardware* badge, write-only key
  entry (store/replace/forget — never read back), removal for custom
  rows, and a **Sign in with Claude** button on the Anthropic row that
  stays disabled *with the reason* until Anthropic publishes registerable
  OAuth endpoints (rule 8: the app never pretends). Each row expands to
  show its **model routing** (`remote:<id>:<model-id>`) and the
  provider's own notes — the broker doesn't enumerate a provider's models
  (that needs egress + a key) and we never invent a model list.
  Below the list, **What may leave this machine**: per-scope switches
  (`prompt`, `files`, `mail`, `calendar`, `screen`, `memory`), default
  all off; a banner states the measured condition. Broker unreachable →
  defaults shown, switches inert.

## Layout

- `lisa-settings.js` — GTK4/libadwaita app (GJS, ESM). Providers/consent
  are async D-Bus with a graceful offline mode; local models come from
  the `lisa` CLI via `Gio.Subprocess` (one source of truth for §8 fit
  logic — Rust, not reimplemented in JS).
- `lib/model.js` — pure view-model: broker-state parsing with safe
  defaults, provider/consent rows, sign-in gating, form validation,
  **plus** catalog parsing, local-model rows + fit badges, profile
  summary, and per-provider routing help. No GTK imports.
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
