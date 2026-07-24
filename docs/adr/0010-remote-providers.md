# ADR-0010: BYO remote model providers via a dedicated egress broker (`lisa-remoted`)

- **Status:** accepted
- **Date:** 2026-07-22

## Context

PLAN §5.11 allows *optional third-party endpoints* — user-supplied, never
Lisa-operated — with every request "rendered in the Ledger and status UI
in a distinct 'leaves your hardware' color". CLAUDE.md rule 5 is
load-bearing: `lisa-inferenced` never gets network access; today only
`lisa-modeld` does (model traffic). Cloud inference therefore cannot be
implemented inside `inferenced`, and bolting it onto `modeld` would
conflate two very different egress policies (pinned artifact mirrors vs.
user-chosen inference APIs).

The user wants bring-your-own accounts for OpenAI, Anthropic (including
a "Sign in with Claude" OAuth path), Thinking Machines' Tinker,
Together.ai, Fireworks.ai, and arbitrary OpenAI-compatible URLs — plus a
desktop Settings surface to manage them and per-scope consent for what
context may leave the device.

## Decision

### 1. A second (and last) egress-permitted daemon: `daemons/remoted`

`lisa-remoted` is the **only** component that talks to provider APIs. It
mirrors `modeld`'s position in the architecture: a small, separately
sandboxed daemon whose systemd unit permits network egress, while
`inferenced`/`contextd`/`agentd` keep `IPAddressDeny=any`. `inferenced`
reaches the broker over a **local unix socket** (`AF_UNIX` is already in
its `RestrictAddressFamilies`) — `inferenced` gains no network, no new
address families, no DNS.

Name: `remoted` follows the `-d` daemon convention (`inferenced`,
`modeld`, `contextd`, `agentd`) and says exactly what it governs: the
*remote* tier of §5.11 (`remote:personal` later rides the same surface;
this ADR delivers `remote:byo`).

Interfaces:

- **Unix-socket HTTP (axum)** — the data plane. `POST
  /v1/chat/completions` in the OpenAI-compatible shape `inferenced`
  already speaks, with two Lisa headers: `x-lisa-provider` (registry id)
  and `x-lisa-scopes` (comma-separated context scopes the request
  carries). Admin endpoints (`/v1/providers`, `/v1/consent`,
  `/v1/oauth/...`) share the socket; socket permissions are the access
  control (0700 runtime dir, M2 attaches per-app identity via the
  portal + `SO_PEERCRED` like §5.1).
- **D-Bus `dev.lisaos.Remote1`** — the management plane for the Settings
  app: list/add/remove providers, set/clear credentials, per-scope
  consent switches, and the Sign-in-with-Claude OAuth start/finish.
  Tested over zbus p2p (macOS + CI), registered on the bus on real
  systems.

### 2. Data-driven provider registry

Providers are rows in a table, not code: `{id, display_name, base_url,
auth style, dialect, notes}`. Two dialects exist:

- `openai-compat` — request body forwarded verbatim to
  `{base_url}/chat/completions`, `Authorization: Bearer <key>`.
- `anthropic-messages` — native `POST {base_url}/v1/messages` with
  `x-api-key` + `anthropic-version: 2023-06-01`; the broker translates
  to/from the OpenAI-compatible shape (system-message hoisting, content
  block flattening).

Anthropic *does* publish an OpenAI-compat layer at
`https://api.anthropic.com/v1/` but documents it as test-only and it
drops guaranteed schema conformance (`strict`/`response_format`
ignored) — since guided generation is a core Lisa primitive (§5.1), the
compat layer would lie to us. Anthropic is therefore a native-dialect
provider ([OpenAI SDK compatibility — Claude Platform Docs](https://platform.claude.com/docs/en/api/openai-sdk)).

Built-in registry rows (endpoints verified against provider docs per
CLAUDE.md rule 8; sources noted inline in `registry.rs`):

| id | base_url | auth | dialect |
|---|---|---|---|
| `openai` | `https://api.openai.com/v1` | Bearer | openai-compat ([API reference](https://developers.openai.com/api/reference/overview)) |
| `anthropic` | `https://api.anthropic.com` | `x-api-key` or OAuth Bearer | anthropic-messages ([Authentication](https://platform.claude.com/docs/en/manage-claude/authentication)) |
| `tinker` | `https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1` | Bearer | openai-compat ([Tinker docs](https://tinker-docs.thinkingmachines.ai/tinker/compatible-apis/openai/)) |
| `together` | `https://api.together.ai/v1` | Bearer | openai-compat ([Together docs](https://docs.together.ai/docs/openai-api-compatibility)) |
| `fireworks` | `https://api.fireworks.ai/inference/v1` | Bearer | openai-compat ([Fireworks docs](https://docs.fireworks.ai/tools-sdks/openai-compatibility)) |

Custom OpenAI-compat providers (a URL the user supplies, §5.11) are
persisted rows in `providers.toml` under the broker's state dir with the
same shape; no code per provider.

**Tinker** is a full inference provider: its sampling surface includes a
documented (beta) OpenAI-compatible API over hosted and fine-tuned
models (`/chat/completions`, `/completions`; models addressed by
`tinker://` checkpoint URIs). The same credential also serves the M6
adapter-training lane (Tinker is primarily a fine-tuning API), so the
broker both proxies inference through it *and* holds the key for later
training tooling to fetch via the management plane.

### 3. Credential storage

There is no existing secrets pattern in the repo (contextd's
encryption-at-rest arrives with the keyring work). Decision: a
**mode-0600 file per credential in a mode-0700 state directory** owned
by the daemon user — `$STATE_DIRECTORY/keys/<provider>.key` (systemd
`StateDirectory=lisa/remoted`, `DynamicUser`), falling back to
`~/.local/share/lisa/remoted` in dev. Justification: boring (rule 4),
auditable with `ls -l`, no new daemon dependency, and consistent with
the repo's "user can open everything with standard tools" stance —
while keeping keys out of the Ledger and out of any world-readable
path. OAuth tokens are stored the same way
(`keys/<provider>.oauth.json`). systemd `LoadCredential`/TPM sealing is
a Track-I hardening follow-up, not a v1 dependency. Keys are write-only
through every API surface: the broker reports *presence*, never value.

### 4. "Sign in with Claude" (Anthropic OAuth)

What Anthropic's public documentation supports today:

- OAuth bearer tokens are sent as `Authorization: Bearer <token>` plus
  the header `anthropic-beta: oauth-2025-04-20` (endpoint-dependent but
  required for `/v1/messages`).
- An interactive OAuth login exists (the `ant auth login` flow:
  browser authorize → code paste with `--no-browser`), i.e. an
  authorization-code flow; OAuth 2.1 / PKCE `S256` is the current
  practice for such flows.
- **No public client registration is documented.** Anthropic's docs
  list API keys and Workload Identity Federation as the supported
  authentication methods for third-party software, and community
  reports indicate OAuth use is restricted to Anthropic's own clients
  by the Consumer ToS.

Per rule 8 the broker implements the *machinery* — PKCE
verifier/challenge (RFC 7636 S256), authorize-URL construction,
form-encoded code exchange, refresh, token storage, and the
`oauth-2025-04-20` bearer header on requests.

**Update (2026-07-24): the constants are no longer unset.** They are the
*verified public* client IDs and endpoints of a shipping OAuth public
client (Construct, `brain/oauth/`), not guessed values — so rule 8 is
satisfied by citation, not by leaving the fields blank:

- **Anthropic (Claude Pro/Max):** client `9d1c250a-e61b-44d9-88ed-5944d1962f5e`,
  authorize `https://claude.ai/oauth/authorize`, token
  `https://platform.claude.com/v1/oauth/token`, loopback
  `http://localhost:53692/callback`, scopes `user:profile user:inference`.
  Requests also carry `?beta=true` on `/v1/messages` and the Claude-Code
  system marker (both required, or the API returns 429).
- **OpenAI (ChatGPT Plus/Pro, "codex"):** client
  `app_EMoamEEZ73f0CkXaXp7hrann`, authorize/token under
  `https://auth.openai.com/oauth/...`, loopback
  `http://localhost:1455/auth/callback`, scopes
  `openid profile email offline_access`, `originator=lisa`.

The broker (only daemon with egress) runs the whole flow on a
**loopback-only** callback server; the Settings panel opens the authorize
URL and reacts to `LoginCompleted`. A successful sign-in is a Ledgered
`remote.grant` (egress capability); logout is the matching revoke. Only
`anthropic` and `openai` are `oauth_capable`; every other provider stays
key-only. API-key auth for Anthropic still works regardless, and takes
second place to OAuth when both are present. Caveat: a ChatGPT
subscription token authenticating the OpenAI *chat-completions* API is
account-dependent — sign-in can succeed while requests are refused
downstream; the broker surfaces that rather than masking it.

### 5. Ledger + consent: nothing leaves silently, nothing leaves by default

- **Every** remote request writes a Ledger entry *before* egress
  (dataflow rule 4: append failure → the request is refused), kind
  `remote.generate`, `model` = `"<provider>:<model>"`, and a `detail`
  JSON carrying `{"egress":"remote","provider":...,"scopes":[...]}`.
  Completion/denial writes `remote.complete` with status/tokens. The
  `remote.` kind prefix is the machine-readable "leaves your hardware"
  marking; UIs render it in the dedicated egress color.
- **Egress color:** `#E66100` (amber/orange) with CSS class
  `leaves-hardware` — warm and unmistakable against the neutral
  palette; the Settings app ships it now, the Ledger app and status UI
  adopt the same class/kind-prefix in their own lanes.
- **Per-scope offload consent** (§5.11 "may offload" switch): scopes
  `prompt`, `files`, `mail`, `calendar`, `screen`, `memory` — matching
  the §5.3 context sources. All default **off**; even `prompt` must be
  explicitly enabled before any request is proxied. A request declaring
  a scope that is not enabled is refused (and the refusal is ledgered).
  Enforcement lives in the broker, not in callers.

### 6. Field-test key provisioning over the ESP (provisional)

Until the M7 OOBE exists, the only partition writable from a macOS dev
machine on the field iMac is the FAT ESP. Staging convention:
`ESP:/lisa-provision/<provider>.key` (e.g. `tinker.key`,
`together.key`, `fireworks.key`, `openai.key`, `anthropic.key`). A
oneshot unit (`lisa-remoted-provision.service`, `Before=` the broker,
`ConditionPathIsDirectory=/efi/lisa-provision`) runs `lisa-remoted
--import-esp /efi`, which:

1. imports each `<provider>.key` whose stem matches a registry id into
   the 0600 secrets store;
2. **removes it from the ESP** — FAT is world-readable, so the ESP is a
   staging area, never the store. The file is overwritten with zeros
   and fsynced before unlink (best-effort on FAT; the honest guarantee
   is "no longer present", not forensic erasure), and the
   `lisa-provision` directory is removed when emptied.

This mechanism is explicitly provisional and is superseded by the M7
installer OOBE.

### 7. `inferenced` surface

`inferenced` gains only config: a `[remote]` section (`enabled`,
default false; `socket` path) and recognition of the
`remote:byo:<provider>:<model>` model-hint prefix so routing can hand
such requests to the broker socket in the follow-up wiring PR. No
network code, no new dependencies, sandbox unchanged.

## Consequences

- The egress story stays auditable: exactly two units may egress
  (`modeld`: pinned model mirrors; `remoted`: user-configured provider
  hosts), and `tests/e2e/egress-test.sh`'s guarantees for the no-egress
  daemons are untouched. A pinned-host firewall for `remoted` (nftables
  set derived from the registry) is follow-up hardening.
- Adding a provider is a registry row (or a user action in Settings),
  not a patch.
- The Ledger gains its first `remote.*` kinds; the append-only schema
  needs no migration (kind is free-form).
- Sign in with Claude is honest: implemented, verifiable, and inert
  until real endpoints exist — no invented URLs shipped.
- The ESP provisioning path trades forensic-grade erasure for
  practicality on FAT; acceptable for field-test keys, retired at M7.
- Packaging: unit files land in `os/packages/lisa/`; PKGBUILD wiring
  waits for the desktop-lane PR that owns that file (noted in the PR).
