# ADR-0005: License the project GPL-2.0-only

- **Status:** accepted
- **Date:** 2026-07-21

## Context

The repo went public at M0 with no license, which made contributions and
forks legally murky. Decision from the project owner: use the same
license as the Linux we fork.

## Decision

**GPL-2.0-only** — the Linux kernel's license — for all first-party code
in this monorepo (`LICENSE` at the root, `license = "GPL-2.0-only"` in
the Cargo workspace). Model weights are not covered: each catalog entry
carries its own reviewed license (PLAN §7), surfaced before download.

## Consequences

- Copyleft matches the substrate we ship and Arch tooling conventions
  (pacman is GPL-2.0-or-later; GPL-2.0-only code composes with it).
- SDK consumers link liblisa via C ABI/GI; if a more permissive SDK
  license (LGPL/MIT) proves necessary for app adoption, that's a
  follow-up ADR scoped to `libs/` — not assumed now.
- Contributions are accepted under the same license (inbound=outbound).
