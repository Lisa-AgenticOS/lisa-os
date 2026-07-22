# lisa-remoted — BYO remote-provider egress broker

Spec: `docs/PLAN.md` §5.11 (optional third-party endpoints) ·
Decision record: `docs/adr/0010-remote-providers.md`.

The one component besides `lisa-modeld` with network access. Everything
else keeps rule 5: `lisa-inferenced` reaches this broker over a local
unix socket and gains no network itself.

## What it does

- **Provider registry (data, not code):** built-in verified rows —
  `openai`, `anthropic` (native Messages API; their OpenAI-compat layer
  is documented test-only and drops schema conformance), `tinker`
  (Thinking Machines, OpenAI-compat sampling beta), `together`,
  `fireworks` — plus user-supplied OpenAI-compat URLs persisted in
  `providers.toml`.
- **Credentials:** one mode-0600 file per key in a 0700 state dir
  (`keys/<provider>.key`); write-only through every API surface.
- **Consent:** per-scope "may offload" switches (`prompt`, `files`,
  `mail`, `calendar`, `screen`, `memory`), all default **off** — by
  default nothing leaves the device, not even the prompt.
- **Ledger:** a `remote.generate` entry precedes every egress (no
  entry, no request); completions/denials land as `remote.complete` /
  `denied`. The `remote.` kind prefix is the machine-readable "leaves
  your hardware" marking; UIs render it in the egress color `#E66100`.
- **Sign in with Claude:** full PKCE (S256) machinery, endpoints
  **explicitly unset** until Anthropic publishes a registerable client
  (CLAUDE.md rule 8 — no invented URLs). API keys work today.
- **ESP provisioning (field test, provisional):** `--import-esp <mnt>`
  imports staged `lisa-provision/<provider>.key` files into the 0600
  store and scrubs them off the world-readable FAT ESP. Shipped as the
  `lisa-remoted-provision.service` oneshot; superseded by the M7 OOBE.

## Interfaces

- Unix-socket HTTP: `POST /v1/chat/completions` (OpenAI-compat body +
  `x-lisa-provider`, `x-lisa-scopes` headers); management under
  `/v1/providers`, `/v1/consent`, `/v1/oauth/claude/*`; `GET /health`.
- D-Bus `org.lisa.Remote1` (management plane for Settings): `State`,
  `AddProvider`, `RemoveProvider`, `SetKey`, `ClearKey`, `SetConsent`,
  `ClaudeOauthStart`, `ClaudeOauthFinish`. Tested over zbus p2p.

## Run (dev)

```sh
cargo run -p lisa-remoted -- --state-dir /tmp/lisa-remoted \
    --ledger /tmp/lisa-remoted/ledger.db
# oneshot ESP import:
cargo run -p lisa-remoted -- --state-dir /tmp/lisa-remoted --import-esp /Volumes/ESP
```

Units: `os/packages/lisa/lisa-remoted.service`,
`os/packages/lisa/lisa-remoted-provision.service`.
