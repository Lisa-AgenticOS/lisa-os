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
- **Credentials:** one mode-0600 file per credential in a 0700 state dir
  — API keys as `keys/<provider>.key`, OAuth sessions as
  `keys/<provider>.oauth.json` (`{"type":"oauth","refresh":…}`);
  write-only through every API surface (only the refresh is persisted —
  access tokens live in memory).
- **Consent:** per-scope "may offload" switches (`prompt`, `files`,
  `mail`, `calendar`, `screen`, `memory`), all default **off** — by
  default nothing leaves the device, not even the prompt.
- **Ledger:** a `remote.generate` entry precedes every egress (no
  entry, no request); completions/denials land as `remote.complete` /
  `denied`. The `remote.` kind prefix is the machine-readable "leaves
  your hardware" marking; UIs render it in the egress color `#E66100`.
  Streamed requests hit the same gate: `remote.generate` before the
  first byte leaves, `remote.complete` when the stream ends — ok,
  error, idle timeout, or consumer disconnect (`aborted`) — with the
  accumulated token/char counts.
- **True streaming:** a data-plane request with `stream:true` streams
  the provider's SSE back over the unix socket as `text/event-stream`
  (`data:` chunk frames, `data: [DONE]` terminator). Anthropic
  Messages events (`message_start`, `content_block_delta`,
  `message_delta`, `message_stop`) are translated on the fly to the
  OpenAI `chat.completion.chunk` shape, so consumers see one format
  regardless of dialect; mid-stream failures arrive as a
  `{"error":...}` frame before `[DONE]`. A provider that goes silent
  mid-stream is cut after 120 s idle. `stream:false` behaves as before.
- **Sign in with Claude / ChatGPT (OAuth):** browser-callback flow with
  RFC 7636 PKCE (S256). `BeginLogin` binds a loopback callback server on
  the provider's fixed port (127.0.0.1:53692 Claude / :1455 ChatGPT),
  returns the authorize URL for the panel to open; on redirect the broker
  exchanges the code, persists the (rotating) refresh token, and emits
  `LoginCompleted`. On chat, OAuth takes precedence over any stored API
  key (Anthropic → `Authorization: Bearer` + `anthropic-beta:
  oauth-2025-04-20`; OpenAI → plain Bearer). Endpoints/client-ids are
  VERIFIED public constants ported from the shipping Construct app
  (`brain/oauth/`), pinned in `oauth.rs` (CLAUDE.md rule 8 — no invented
  URLs). API keys still work for every provider.
- **ESP provisioning: removed (#164, 2026-08-01).** A `--import-esp`
  oneshot used to exist that read `lisa-provision/<provider>.key` off
  the FAT ESP into the 0600 store and scrubbed the staging file. No
  installer ever shipped the unit, so staged keys were neither imported
  nor scrubbed — they simply sat in plaintext on a world-readable
  partition while the broker reported "no key stored". The code, the
  units and the flag are gone rather than wired up: keys now arrive
  through Settings › Intelligence (API key or OAuth sign-in), which is
  the path that is actually tested on hardware, and staging a plaintext
  secret on an unencrypted partition is not a mechanism worth shipping
  as a supported alternative. `git log -- daemons/remoted/src/provision.rs`
  has the implementation if the ESP route is ever needed again.

## Who may change things (issue #99)

`SetConsent`, `SetKey`, `ClearKey`, `AddProvider`, `RemoveProvider`,
`BeginLogin` and `Logout` are reachable only from a program Lisa ships
for the purpose, running as this daemon's own user. The allowlist is
`lisa_peer::manager::DEFAULT_MANAGERS` — Settings and the two CLI copies
— shared with the portal, because the same hole existed there (#107).

**Reads stay open**: `Ping`, `State`, `ListModels`, `GET /health`,
`/v1/providers`, `/v1/consent`, and the whole data plane. `inferenced`
calls the data plane, and what it may send is governed by the offload
scopes, which are now the thing that cannot be flipped from outside.

Identity comes from the transport, on both planes:

| plane | how |
|---|---|
| `dev.lisaos.Remote1` (session bus) | `GetConnectionCredentials` → pidfd → `/proc/<pid>/exe` |
| `remoted.sock` | `SO_PEERCRED` + `SO_PEERPIDFD` → `/proc/<pid>/exe` |

Before this there was no check of any kind. The socket's 0600 mode was
described as the access control, and it is a real defence against
*another user* — but the threat is another **process of the same user**:
an app you installed, a Flatpak with session access, something the agent
built. Any of them could `PUT /v1/consent` six times, turn on every
offload scope, and then proxy `mail`, `files`, `screen` and `memory`
content out through the broker. The only trace was one `remote.consent`
row attributed to `settings` — so the audit trail actively blamed the
panel. Management entries now name the program the kernel reports.

### Where this degrades

Naming a program needs a pidfd. Without one — a kernel before 6.5, a
D-Bus broker before 1.16, a non-Linux host — nothing can be identified
and **every management call is refused**. That is fail-closed and
intentional, and both branches are asserted rather than skipped:
`LISA_REQUIRE_PIDFD=1` makes the unidentifiable case a test failure, and
CI sets it in the job whose base is new enough.

### The limit, stated plainly

An allowlisted program is trusted completely. This moves the boundary
from "any process on your session bus" to "three files"; it is not the
same as proving those three files never misbehave. A per-action
confirmation for consent flips — the switch PLAN §5.11 rests on — is
still worth having and is not here.

## Interfaces

- Unix-socket HTTP: `POST /v1/chat/completions` (OpenAI-compat body +
  `x-lisa-provider`, `x-lisa-scopes` headers); management under
  `/v1/providers`, `/v1/consent`, `POST /v1/oauth/{provider}/begin`,
  `DELETE /v1/oauth/{provider}`; `GET /health`.
- D-Bus `dev.lisaos.Remote1` (management plane for Settings): `State`,
  `AddProvider`, `RemoveProvider`, `SetKey`, `ClearKey`, `SetConsent`,
  `BeginLogin`, `Logout`, `ListModels`, and the `LoginCompleted` signal.
  Each `State` provider row carries `auth` (`"oauth"`/`"key"`),
  `oauth_capable`, and `connected`. Tested over zbus p2p.

## Run (dev)

```sh
cargo run -p lisa-remoted -- --state-dir /tmp/lisa-remoted \
    --ledger /tmp/lisa-remoted/ledger.db
```

Unit: `os/packages/lisa/lisa-remoted-user.service`, installed by the
PKGBUILD as `usr/lib/systemd/user/lisa-remoted.service`. The broker runs
per-user, not system-wide — Settings and the CLI talk to the user
instance, so a system-scope copy would hold state nobody reads (#164).

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
