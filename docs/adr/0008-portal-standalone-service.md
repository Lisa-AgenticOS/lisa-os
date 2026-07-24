# ADR-0008: The Lisa portal is a standalone session service, consent stays in the shell

- **Status:** accepted
- **Date:** 2026-07-22

## Context

PLAN §5.5 specifies `xdg-desktop-portal-lisa` — per-app identity,
consent, quotas, and Ledger attribution — "Rust or C matching upstream
portal conventions", with the interfaces "drafted as a freedesktop
proposal". Upstream xdg-desktop-portal has a two-process shape: a
frontend (`org.freedesktop.portal.Desktop`) that sandboxed apps may
always reach, and per-desktop backend implementations
(`org.freedesktop.impl.portal.*`) that own the dialogs. Adding a *new*
frontend interface, however, means patching and shipping our own
xdg-desktop-portal build — a heavy fork to carry before the interface
has even stabilized, and rejected by the same logic as ADR-0002's
"boring tech" rule: keep the novelty budget on the product surface.

## Decision

1. **Ship the portal as its own session D-Bus service** —
   `dev.lisaos.Portal` at `/dev/lisaos/portal/desktop`, Rust/zbus (ADR-0002),
   D-Bus-activated, systemd `--user` unit. Interfaces:
   `dev.lisaos.portal.Inference` + `dev.lisaos.portal.Session` (M2, live),
   `dev.lisaos.portal.Grants` (Settings backend, M2, live),
   `dev.lisaos.portal.{Context,Memory,Agent}` (reserved; M3/M5).
2. **Adopt upstream's frontend/impl split for consent:** the portal
   decides policy; the pixels live in the shell, reached via
   `dev.lisaos.impl.portal.Consent` (`AskConsent(app_id, app_kind, scope)
   → (allow, remember)`, served by `dev.lisaos.Shell`). No dialog service
   → first-use requests are **denied**, never silently allowed.
3. **Identity the upstream way:** Flatpak apps are identified by
   `/proc/<pid>/root/.flatpak-info` (unforgeable from inside the
   sandbox); host apps by peer-cred pid + `.desktop` Exec mapping,
   `host:<comm>` fallback — best effort, recorded as `identity=host` in
   the Ledger.

Until the freedesktop proposal lands, a Flatpak app needs
`--talk-name=dev.lisaos.Portal` in its manifest to reach the portal. That
one visible line replaces a patched portal frontend; the demo app and
SDK templates carry it. When/if GNOME/KDE adopt the interface, the same
objects move behind `org.freedesktop.portal.Desktop` unchanged.

## Consequences

- No xdg-desktop-portal fork to maintain; the interface can iterate
  freely until it is worth proposing upstream.
- The `--talk-name` line weakens "zero special permissions" from the
  §5.5 acceptance wording to "zero permissions beyond one talk-name";
  full zero-permission access requires the upstream frontend and is
  deferred with it.
- The shell (M4 surfaces) owns one more small D-Bus service: the
  consent dialog. Until it ships, real first-use grants can only come
  from `dev.lisaos.portal.Grants` (Settings/CLI pre-grant) or the
  explicit `--consent allow` dev mode.
- `agentd` (M5) consumes this precedent: `dev.lisaos.portal.Agent` lands
  on the same bus name, grant store, and consent path.
- The portal session ledger lives per-user (`~/.local/share/lisa/`);
  unifying it with the system daemons' StateDirectory ledger is a
  Ledger-app concern (§5.7.6), not a portal one.
