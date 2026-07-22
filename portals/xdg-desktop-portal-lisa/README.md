# xdg-desktop-portal-lisa — the trust boundary

Spec: docs/PLAN.md §5.5 (security model §5.10). Milestone: M2.
Shape: ADR-0008 — a standalone session D-Bus service (`org.lisa.Portal`),
not an xdg-desktop-portal fork; consent pixels live in the shell.

Sandboxed apps never talk to the Lisa daemons directly (PLAN §4 rule 1).
This portal is the sole door: it attaches per-app identity, runs
first-use consent, enforces per-app quotas, writes every decision and
call to the Ledger under the real app id, and proxies inference sessions
to `org.lisa.Inference1` so revoking a grant kills the live session.

## D-Bus surface

Bus name `org.lisa.Portal`, object `/org/lisa/portal/desktop`, session
bus, D-Bus-activated (`os/packages/lisa/org.lisa.Portal.service` +
systemd user unit).

**`org.lisa.portal.Inference`**
- `Ping() → s` — liveness.
- `OpenSession(options a{sv}) → (session o, fd h)` — identity →
  consent → Ledger → proxied daemon session. `options` are forwarded to
  `org.lisa.Inference1.OpenSession` (`model_hint`, …); the portal adds
  `app_id`. The fd is the daemon's token pipe, passed through untouched
  (EOF = end of message, exactly as in §5.1).

**`org.lisa.portal.Session`** (returned object)
- `Generate(prompt s, params a{sv})` — quota gate (requests/min +
  tokens/day) → ledger entry under the app id → daemon `Generate`.
  Params forwarded: `schema` (guided generation), `max_tokens`,
  `priority`.
- `Embed(texts as) → aad`, `Cancel()`, `Close()` — same gates.

**`org.lisa.portal.Grants`** (the Settings › Intelligence backend;
host-only — sandboxed callers are refused, apps cannot grant themselves)
- `List() → a(sss)` — (app_id, scope, "allowed"|"denied"|"unset").
- `Grant(app_id s, scope s)` / `Deny(app_id s, scope s)` — pre-set a
  decision.
- `Revoke(app_id s, scope s) → u` — record the revoke, kill every live
  session under the grant (daemon session closed → the app's fd sees
  EOF; portal object removed) and return the count. Next request
  prompts again. §5.5 acceptance: this lands in well under 1 s.

`org.lisa.portal.{Context,Memory,Agent}` (§5.5) are reserved interface
names; they land with M3/M5 on the same grant store and consent path.

## Identity

- **Flatpak (strong):** `/proc/<pid>/root/.flatpak-info` `[Application]
  name` — the upstream portal mechanism; unforgeable from inside the
  sandbox.
- **Host (best effort, documented as weaker):** peer-cred pid →
  `/proc/<pid>/comm` → `.desktop` Exec mapping; fallback `host:<comm>`.
  Ledger entries carry `identity=host`.

Until the freedesktop frontend proposal lands, Flatpak apps need
`--talk-name=org.lisa.Portal` (ADR-0008).

## Consent

First-use grant with "always / only this time"; remembered allows and
denies never re-prompt; revoke returns the pair to first-use. The
dialog itself is the shell's: `org.lisa.Shell` serving
`org.lisa.impl.portal.Consent` at `/org/lisa/impl/portal/consent`,
`AskConsent(app_id s, app_kind s, scope s) → (allow b, remember b)`.
No dialog service reachable → **deny** (fail closed). Dev modes:
`--consent allow|deny`.

## Grants, quotas, Ledger

- Grant store: append-only action log (SQLite, UPDATE/DELETE aborted by
  triggers — same construction as the Ledger), per-user at
  `~/.local/share/lisa/grants.db` (`$LISA_GRANTS_DB`). Effective state
  is derived; `allow_once` is logged but never persists.
- Quotas (anti-abuse, not monetization): requests/min (default 120,
  sliding window) and tokens/day (default 500 000, persisted across
  restarts; word-count estimate until inferenced emits TokenUsage).
- Ledger: per-user `~/.local/share/lisa/ledger.db` (`$LISA_LEDGER_DB`);
  no ledger entry, no session (PLAN §4 rule 4). Kinds written:
  `context.grant`, `inference.session`, `inference.generate`,
  `inference.embed` — all under the resolved app id.

## Status

**M2 core implemented and tested** (grant/quota/identity/consent logic
unit-tested host-independently; the D-Bus surface exercised over zbus
p2p, including end-to-end through the real `org.lisa.Inference1`
interface). Still open for the full §5.5 acceptance block: the Flatpak
demo app + live-desktop run (needs the Linux desktop session), the
shell consent dialog (M4 surface), and Settings UI. Run locally:
`xdg-desktop-portal-lisa --upstream stub --consent allow`.
