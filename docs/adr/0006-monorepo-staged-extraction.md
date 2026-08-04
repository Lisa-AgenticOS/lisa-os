# ADR-0006: Monorepo with staged extraction

- **Status:** accepted — extended, not superseded, by ADR-0039: none of this
  ADR's own four triggers has fired; the two that fired are ones it could
  not have contained.
- **Date:** 2026-07-21

## Context

The move to the Lisa-AgenticOS org raised the question of splitting the
monorepo into per-component repos now. PLAN §9 specifies a monorepo; the
project is mid-M0 with one contributor, four Rust crates, and stub
directories for everything else. Milestone acceptance blocks cut across
components (daemon + CLI + SDK + portal), and a typical commit today
touches packaging, units, installers, tests, and CI atomically.

## Decision

**Stay monorepo for the OS core. Split by exception, on triggers — not
on a date.**

| Stage | Extracted repo | Trigger |
|---|---|---|
| 1 | `catalog` (model catalog data + signing) | Catalog goes live (M1) — PLAN §6 gives model updates their own release channel; signed data with daily refresh cadence |
| 2 | `liblisa` SDK + bindings + SDK docs | First external consumer / crates.io publication (M2) |
| 3 | `lisa_ui`, `lisa_flutter`, `forge` | Flutter lane becomes real (M6): different toolchain, community app lane |
| 4 | `themes`, `fcitx5-lisa`, portal spec | Community theme engine (Appendix E); upstreaming to fcitx5 / freedesktop |

**Never split:** daemons, portal, CLI, `os/*`, `tests/*`, and
`docs/PLAN.md` + ADRs — this *is* the OS; its acceptance gates span
these components and must remain single-commit-testable.

Extraction mechanics when a trigger fires: `git filter-repo` so the
component keeps its history; the org provides the landing spot; the
monorepo consumes the extracted piece via its release artifacts (signed
catalog, published crate), never via git submodules.

## Consequences

- Cross-cutting milestone work stays atomic; one commit passes an
  acceptance gate or doesn't.
- The usual motivation for splitting — CI cost — is addressed instead
  with per-job path filters in CI (docs-only commits skip the heavy
  jobs).
- Precedent: systemd ships ~70 binaries from one repo; Omarchy is one
  repo; SteamOS keeps its delta small. Multi-repo suits multi-team
  projects with release engineering to spare, which we are not.
- Each trigger firing gets a short ADR appendix here noting the
  extraction, rather than a new ADR.

## Appendix: extractions

- **2026-08-02** — two extractions, on triggers this ADR could not have
  contained (they were created by later decisions), and defining what
  the extracted pieces are consumed *as* (a pacman package in a hosted
  `[lisa]` repo) was more than an appendix could carry — hence
  ADR-0039, the one exception to the "no new ADR" rule above, for the
  reasons given there. Extracted: **`lisa-desktop`** (`shell/*`,
  `ime/*`; trigger: ADR-0038's vendored GNOME Shell fork — `ime/` rides
  along as part of the desktop surface, stage 4's "upstreaming to
  fcitx5" trigger has not fired) and **`lisa-apps`** (`apps/*` less
  `apps/notes`; trigger: ADR-0020's image-independent app channel —
  `apps/*` appears in none of this ADR's stages). All four staged
  triggers above remain unfired and are recorded as held in ADR-0039:
  no live catalog channel, no external `liblisa` consumer, no shipped
  Flutter app, nothing upstreamed.
