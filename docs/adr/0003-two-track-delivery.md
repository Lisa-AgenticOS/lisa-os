# ADR-0003: Two-track delivery — Lisa Layer first, immutable image as the product

- **Status:** accepted — both tracks ship: Track L installs onto stock Arch
  from the signed `[lisa]` index, Track I is the released image.
- **Date:** 2026-07-20
- **Claims:**
  - `path:os/layer/install.sh` — Track L, onto stock Arch
  - `symbol:\[lisa\]@os/layer/install.sh` — from the signed index
  - `path:os/mkosi/mkosi.conf` — Track I, the released image

## Context

Omarchy's growth path (install script on stock Arch → own ISO → hardware
partnerships in under a year) is the strongest existence proof for
shipping an opinionated Arch derivative (PLAN §3, Appendix E). Building
the immutable image first would delay dogfooding by months.

## Decision

- **Track L — the Lisa Layer (first, fast):** every §5 component
  packaged as a pacman repo + one install script targeting stock Arch
  *and Omarchy itself* (`os/layer/`). Rollback via Btrfs + Snapper
  pre-update snapshots with Limine boot-menu restore, adopting Omarchy's
  hard-won config: snapshot `/` only, never `/home`; btrfs quotas off.
- **Track I — the immutable image (the product):** the mkosi/UKI/A-B
  system of ADR-0001 (`os/mkosi/`), which Track L's packages drop into
  unchanged. The full §5.10 security story (measured egress, dm-verity)
  is only fully enforceable here; Track I remains the destination.

## Consequences

- Dogfooding in weeks, and Omarchy's install base — Arch developers on
  modern hardware — becomes our beachhead ecosystem, not a competitor.
- We maintain two update stories during overlap (pacman hooks vs.
  sysupdate); the pacman repo is shared infrastructure for both.
- Ledger/egress guarantees are documented as *degraded* on Track L
  (mutable root) — honesty about the difference is part of the product.
