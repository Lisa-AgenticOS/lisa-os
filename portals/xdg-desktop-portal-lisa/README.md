# xdg-desktop-portal-lisa — the trust boundary

Spec: docs/PLAN.md §5.5 (security model §5.10). Milestone: M2.
Shape: ADR-0008 — a standalone session D-Bus service (`dev.lisaos.Portal`),
not an xdg-desktop-portal fork; consent pixels live in the shell.

Sandboxed apps never talk to the Lisa daemons directly (PLAN §4 rule 1).
This portal is the sole door: it attaches per-app identity, runs
first-use consent, enforces per-app quotas, writes every decision and
call to the Ledger under the real app id, and proxies inference sessions
to `dev.lisaos.Inference1` so revoking a grant kills the live session.

## D-Bus surface

Bus name `dev.lisaos.Portal`, object `/dev/lisaos/portal/desktop`, session
bus, D-Bus-activated (`os/packages/lisa/dev.lisaos.Portal.service` +
systemd user unit).

**`dev.lisaos.portal.Inference`**
- `Ping() → s` — liveness.
- `OpenSession(options a{sv}) → (session o, fd h)` — identity →
  consent → Ledger → proxied daemon session. `options` are forwarded to
  `dev.lisaos.Inference1.OpenSession` (`model_hint`, …); the portal adds
  `app_id`. The fd is the daemon's token pipe, passed through untouched
  (EOF = end of message, exactly as in §5.1).

**`dev.lisaos.portal.Session`** (returned object)
- `Generate(prompt s, params a{sv})` — quota gate (requests/min +
  tokens/day) → ledger entry under the app id → daemon `Generate`.
  Params forwarded: `schema` (guided generation), `max_tokens`,
  `priority`.
- `Embed(texts as) → aad`, `Cancel()`, `Close()` — same gates.

**`dev.lisaos.portal.Grants`** (the Settings › Intelligence backend;
**manager-only** — see *Who may manage grants* below. Apps cannot grant
themselves, and cannot grant, deny or list anything for anyone else)
- `List() → a(sss)` — (app_id, scope, "allowed"|"denied"|"unset").
- `Grant(app_id s, scope s)` / `Deny(app_id s, scope s)` — pre-set a
  decision.
- `Revoke(app_id s, scope s) → u` — record the revoke, kill every live
  session under the grant (daemon session closed → the app's fd sees
  EOF; portal object removed) and return the count. Next request
  prompts again. §5.5 acceptance: this lands in well under 1 s.

`dev.lisaos.portal.{Context,Memory,Agent}` (§5.5) are reserved interface
names; they land with M3/M5 on the same grant store and consent path.

## Identity

Who is calling is decided by the transport and the kernel, never by the
message (`lisa-peer`, ADR-0033).

- **Flatpak:** `/proc/<pid>/root/.flatpak-info` `[Application] name` —
  the upstream portal mechanism.
- **Host:** the caller's executable — `/proc/<pid>/exe`, reached through
  the broker's pidfd so the pid cannot be recycled underneath us —
  matched against the `Exec=` of an installed `.desktop` file. A match
  lends that app's id (`identity=host`); anything else gets its own
  bucket, `host:/path/to/binary` (`identity=unattributed`).
- **Unidentifiable callers** (no pidfd, no `/proc`) are `host:unknown`,
  a shared bucket that can never name an installed app.

Two rules make the host path an attestation rather than a claim:

- **Never `comm`.** A process sets its own `comm` (`PR_SET_NAME`, or
  just `argv[0]`), so the old mapping let any binary rename itself
  `gnome-text-editor` and inherit that app's grants, quota and Ledger
  attribution (issue #106).
- **The whole path, never the basename.** Matching `exe`'s basename
  would only move the forgery — an attacker who cannot set `comm` can
  still *name a file*. The caller's executable must be the exact file
  the desktop entry launches, e.g. `/usr/bin/gnome-text-editor`, which
  an unprivileged process cannot create. For the same reason a bare
  `Exec=evince` resolves only against a fixed list of system binary
  directories, not `$PATH` (which routinely includes `~/.local/bin`).

Until the freedesktop frontend proposal lands, Flatpak apps need
`--talk-name=dev.lisaos.Portal` (ADR-0008).

### Who may manage grants

`Grant`/`Deny`/`Revoke`/`List` are reachable only from a program we ship
for the purpose, running as the portal's own user. The allowlist is
three files (`--manager` overrides it):

    /usr/lib/lisa/bin/lisa                                   (baked CLI)
    /var/lib/lisa/apps/payloads/runtime/current/bin/lisa      (channel CLI)
    /usr/bin/gnome-control-center                             (Settings)

`/usr/bin/lisa` is deliberately absent: it is a resolver shell script
that `exec`s one of the two CLI copies, so the shell is never the
caller's executable. Paths are re-resolved at every check, because the
channel copy sits behind a `current` symlink an update moves.

The check the guard replaced was `kind != Flatpak` — which meant every
other process on the session bus could pre-grant a scope to any app
(consent bypass) or write a remembered `Deny` and lock an app out
(issue #107).

An allowlisted program is trusted completely: this moves the boundary
from "any process" to "three files", which is the fix, and is not the
same as proving those three files never misbehave.

## Consent

First-use grant with "always / only this time"; remembered allows and
denies never re-prompt; revoke returns the pair to first-use. The
dialog itself is the shell's: `dev.lisaos.Shell` serving
`dev.lisaos.impl.portal.Consent` at `/dev/lisaos/impl/portal/consent`,
`AskConsent(app_id s, app_kind s, scope s) → (allow b, remember b)`.
No dialog service reachable → **deny** (fail closed). Dev modes:
`--consent allow|deny`.

## Grants, quotas, Ledger

- Grant store: append-only action log (SQLite, UPDATE/DELETE aborted by
  triggers — same construction as the Ledger), per-user at
  `~/.local/share/lisa/grants.db` (`$LISA_GRANTS_DB`). Effective state
  is derived; `allow_once` is logged but never persists.
- Quotas (anti-abuse, not monetization): requests/min (default 120,
  sliding window), tokens/day (default 500 000, persisted across
  restarts), and open sessions per app (default 16). `OpenSession`
  spends a request — it used to be free, so an app could hold unbounded
  daemon sessions, file descriptors and D-Bus objects with the request
  quota set to 1 (issue #111).
- The daily budget is **all-or-nothing and atomic**: a request that
  would not fit in what is left of the day is refused and charged
  nothing, and the read and the add are one transaction. Previously the
  check ran before the add against the *old* total, so a single
  1000-token call sailed through a 5-token budget, and concurrent calls
  both spent the same remainder (issue #114).
- **Output is reserved, not measured.** Tokens stream from `inferenced`
  straight down the app's fd, so the portal cannot count what came back
  — before, it counted nothing, and a two-word prompt could drive an
  unbounded generation. A `Generate` is now charged its prompt plus
  either the `max_tokens` it states or `assumed_output_tokens` (2048).
  Real accounting lands when `inferenced` reports TokenUsage per
  session; until then this is deliberately an over-estimate.
- **Consent cannot be worn down.** A refusal the user did not ask to
  remember is recorded (`deny_once`) without becoming a remembered
  denial, and after three refusals in fifteen minutes an app simply
  stops being able to raise the dialog. Asking without limit does not
  defeat consent, it outlasts the person answering (issue #113).
- Ledger: per-user `~/.local/share/lisa/ledger.db` (`$LISA_LEDGER_DB`);
  no ledger entry, no session (PLAN §4 rule 4). Kinds written:
  `context.grant`, `inference.session`, `inference.generate`,
  `inference.embed` — all under the resolved app id.

## Sessions

Each `OpenSession` returns an object bound to the peer that opened it
(`lisa_peer::Owner`), and every method on it checks that owner first.
Being refused and not existing return the **same** error, so a sweep
cannot use the refusal to learn which sessions are live (ADR-0033 §4).

Paths carry a 128-bit token derived from a per-process secret rather
than a counter. They were `/dev/lisaos/portal/session/{1,2,3,…}`; the
path was never the capability, but there is no reason to hand out a free
enumeration of everyone else's sessions (issue #108).

## Testing

    cargo test -p xdg-desktop-portal-lisa

- `tests/portal.rs` runs over a zbus **p2p socketpair** — no bus daemon,
  so consent, grants, quotas, revocation and the real `Inference1`
  round-trip all run on a macOS dev host.
- `tests/bus.rs` runs against a real `dbus-daemon` with **two client
  connections**, because p2p cannot express the property under test: it
  has exactly one peer, so `PeerId` is `Direct` for everybody and "one
  app must not be able to drive another's session" is not a sentence
  that transport can make. A green p2p suite proved nothing about #108.

  Session ownership needs a broker, not Linux, so it runs anywhere
  `dbus-daemon` does. The manager check additionally needs
  `/proc/<pid>/exe` through a **pidfd**, which only Linux supplies — its
  positive control is `#[cfg(target_os = "linux")]` and runs in CI. On
  macOS every grant-management call is refused for want of credentials,
  which is what the negative control asserts.

  Homebrew's `dbus-daemon` does not print its address promptly (and does
  not honour a custom listener), so on a macOS host these tests **skip**
  after a five-second wait. `LISA_REQUIRE_BUS_TESTS=1` makes skipping
  fatal; CI sets it, so the skip can never be mistaken for a pass.

### A broker without pidfds

`GetConnectionCredentials` gained `ProcessFD` in **dbus 1.16**. Without
it `lisa_peer::exe_of_peer` refuses to name a program — deliberately,
since the alternative is a bare pid that can be recycled (#136) — and
the portal degrades in two visible ways:

- no caller can manage grants, and
- **every host app resolves to the shared `host:unknown` bucket**, so
  they share one grant and one quota.

Flatpak identity is unaffected. The portal logs this at `error` on
startup rather than leaving it to be inferred from behaviour. Lisa OS
ships on Arch (dbus 1.16), so this is a warning about foreign hosts, not
about the target.

The tests assert **both** branches instead of skipping the awkward one.
GitHub's runner image carries dbus 1.14, so the main `test` job
exercises the no-pidfd side; a separate `portal-identity` job runs the
suite in a `debian:trixie` container with `LISA_REQUIRE_PIDFD=1`, which
is where the #106/#107 mechanism is actually proven.

## Status

**M2 core implemented and tested**; the six findings of the adversarial
review (#106, #107, #108, #111, #113, #114) are closed and each has a
test that fails without its fix. Still open for the full §5.5 acceptance
block: the Flatpak demo app + live-desktop run (needs the Linux desktop
session), the shell consent dialog (M4 surface), and Settings UI. Run
locally: `xdg-desktop-portal-lisa --upstream stub --consent allow`.

### Known limits

- **Nothing calls `Grants` yet.** The Settings › Intelligence panel does
  not use it, so the allowlist has no live consumer to have validated
  it against a real desktop — only the CI positive control.
- **`.flatpak-info` is read through `/proc/<pid>/root/`**, the upstream
  mechanism. A host process that can create a mount namespace could in
  principle present a fabricated one and claim a sandboxed app's id.
  That is inherited from upstream's design, is *not* one of the six
  findings above, and is filed rather than quietly fixed.
