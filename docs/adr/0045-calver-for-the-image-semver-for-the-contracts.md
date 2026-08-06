# ADR-0045: CalVer for the image, SemVer for the contracts

- **Status:** accepted — both schemes were already in use; this ADR names
  them and retires the ordinal shorthand that caused the confusion.
- Date: 2026-08-04
- Relates: ADR-0001 (A/B image), ADR-0039 (per-repo packages),
  ADR-0016 (versioned D-Bus interface names)
- **Claims:**
  - `symbol:Name=dev.lisaos.Context1@os/packages/lisa/dev.lisaos.Context1.service` — SemVer on the contract, in the name itself

## Decision

Two kinds of artifact, two schemes — both already in use; this ADR
makes the split deliberate:

1. **The OS image stays CalVer** (`YYYYMMDD.run`). SemVer's promise —
   the major number predicts breakage — is meaningless for an A/B
   image: nobody upgrades through an API, sysupdate swaps a whole
   tested slot, and rollback is automatic. Every release would be an
   invented `+1`. Dates sort monotonically (which is all
   systemd-sysupdate needs), and a date tells a human when the thing
   was built — `20260803.65` locates itself; "v34" located nothing,
   which is the confusion that prompted this ADR. Rolling OSes are
   CalVer country (Ubuntu's `24.04`, SteamOS build IDs) for exactly
   these reasons.
2. **Packages and libraries are SemVer**: `lisa-cli`, `lisa-desktop`,
   `lisa-apps`, the keyring, and every crate (where SemVer is law).
   Packages carry compatibility contracts — `lisa-cli >= 0.2` must
   mean something to a dependent — so the scheme that encodes
   contracts applies. Bumps follow real compatibility changes, not
   release cadence: a package may sit at 0.1.0 across many image
   cuts.
3. **D-Bus interfaces version by name** (`dev.lisaos.Context1` →
   `Context2` on incompatible change), the freedesktop convention —
   effectively a major version with coexistence built in. Unchanged.

## Consequences

- Session shorthand ("v33", "v34") is retired from records; STATUS,
  issues and release notes quote real versions.
- The apps/runtime payloads (`lisa-apps_<ver>`, `lisa-runtime_<ver>`)
  inherit the image's CalVer because they ship per-release — they are
  points in time, not contracts. The PACKAGED apps (`lisa-apps` the
  pacman package) are SemVer. Same code, two delivery shapes, each
  named by what it is.
- When `liblisa` publishes (ADR-0006 stage 2), crates.io enforces the
  right half of this automatically.
