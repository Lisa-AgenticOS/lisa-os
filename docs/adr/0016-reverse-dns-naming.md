# ADR-0016: reverse-DNS identifiers move to the real domains (dev.lisaos.* / app.lisaos.*)

- **Status:** accepted
- **Date:** 2026-07-24

## Context

Every IPC and application identifier in the tree used the `org.lisa.*`
reverse-DNS prefix — D-Bus well-known names, object paths (`/org/lisa/...`),
interface names, GApplication ids, `.desktop` ids, D-Bus activation files, and
the MCP tool manifest. `org.lisa` implies the domain **`lisa.org`**, which the
project does not own. The project's domains are **`lisaos.dev`** (the OS) and
**`lisaos.app`** (marketing). Reverse-DNS should be derived from a domain you
control (project owner, 2026-07-24).

## Decision

Rename all identifiers to the real domains, **split by layer**:

- **OS-level → `dev.lisaos.*`** (from lisaos.dev): the daemons and system IPC —
  `Inference1`, `Agent1`, `Overlay1` (+ `.UI`), `Remote1`/`Remoted`,
  `Portal`/`portal`, the portal impl (`impl.portal.*`), `Shell`. Object paths
  `/org/lisa/...` → `/dev/lisaos/...`.
- **Apps → `app.lisaos.*`** (from lisaos.app): user-facing applications —
  `Assistant`, `LedgerApp`, `Settings`, `notes` (the Notes app's MCP tool
  server), and the `lights` test fixture.

Files named after their id are renamed to match (D-Bus activation requires the
filename to equal the bus name): e.g. `org.lisa.Overlay1.service` →
`dev.lisaos.Overlay1.service`, `org.lisa.Assistant.desktop` →
`app.lisaos.Assistant.desktop`.

**Not touched:** the GNOME Shell extension GSettings schema
`org.gnome.shell.extensions.lisa-overlay` — that is GNOME's namespace, not our
reverse-DNS, and renaming it would break the extension.

## Consequences

- A single cross-cutting rename: server and client of each name change in
  lockstep, so nothing breaks as long as each old id maps to exactly one new id
  everywhere (Rust zbus, GJS shell/backend/extension, the C Settings panel,
  service/desktop files, PKGBUILD install paths, MCP manifest).
- It is a clean break, not a compatibility shim: an image update replaces every
  component at once, so old↔new mismatch never occurs on a running system. There
  is no support for a half-updated system (there never was — updates are atomic
  A/B).
- Verified by `cargo test` + `just shell-test` (D-Bus-name assertions catch
  mismatches) and a final grep for stray `org.lisa`; the C panel compiles in CI.
- Overlay1 stays OS-level (`dev.lisaos.Overlay1`): it is the headless shell
  backend/system service, not a standalone app. `notes` is `app.lisaos.notes`:
  it is the Notes app, exposing tools to the Agent Bus.
