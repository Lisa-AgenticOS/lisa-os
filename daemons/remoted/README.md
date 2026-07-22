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

## Packaging & the socket bridge (TODO — needs Linux verification)

The broker is complete and tested; wiring it into the image is deferred
because the unix-socket permission story spans DynamicUser services and
must be verified on real systemd (not macOS). Design:

- A static `lisa` system group; `lisa-inferenced`, `lisa-remoted`, and
  the login `lisa` user all join it (SupplementaryGroups / sysusers.d).
- Prefer **socket activation**: a `lisa-remoted.socket` unit with
  `ListenStream=/run/lisa/remoted.sock`, `SocketGroup=lisa`,
  `SocketMode=0660`; systemd creates the socket with correct group/mode
  before the daemon starts and passes the fd (small `sd_listen_fds`
  change to `main.rs`). This lets `inferenced` (routing), the Settings
  app, and `lisa remote` all reach it, while egress stays broker-only.
- Then: add `lisa-remoted` + the Settings app to `os/packages/lisa`
  (PKGBUILD), enable `lisa-remoted.service` in `00-lisa.preset`, and add
  a Linux CI job asserting `inferenced` reaches the socket end to end
  (a `remote:mock:*` model routes through a stub broker under the
  packaged perms). Verify on the field iMac.

Until then: the management plane + routing are fully usable in dev where
all components share a user (see `Run (dev)` above and `lisa remote`).
