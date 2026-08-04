# ADR-0041: Package signing and the trust chain

- **Status:** accepted — the `[lisa]` index has published signed since
  2026-08-03, with `lisa-keyring` shipping the pinned key. SigLevel flips
  from Optional to Required one release after devices take the keyring.
- Date: 2026-08-03
- Relates: ADR-0039 (the index), ADR-0034 (no uncontrolled
  dependencies in update paths), lisa-os#171; operational detail lives
  in lisa-packages' README

## Context

ADR-0039 created a hosted pacman repo that devices will eventually
install from. Unsigned, its integrity rests entirely on GitHub serving
the bytes CI uploaded — acceptable for the transition, wrong as an end
state, and fatal to ever having mirrors or torrents (#27): a mirror of
unsigned packages is an invitation.

Signing needs a key, and a key needs custody. That is an owner
decision — a script must not improvise it — and it blocked
`SigLevel = Required` for a day until the owner delegated the call.

## Decision

1. **A dedicated key signs packages and nothing else.** *Lisa OS
   Package Signing <packages@lisaos.dev>*, ed25519, fingerprint
   `737240D11D28E109A474A8E5827E27417AF5982B`. Never a personal key:
   the thing that can bless a package every Lisa machine trusts must
   be revocable without revoking a person.
2. **Custody is two copies, no more.** The private half lives in
   `lisa-packages`' `LISA_SIGNING_KEY` Actions secret (so publishing
   stays one dispatch) and in the owner's password manager (because
   GitHub secrets are write-only — lose the second copy and the only
   path is re-keying every device). The public half is committed to
   `lisa-packages` and shipped to devices by `os/packages/lisa-keyring`
   in archlinux-keyring form.
3. **Two-phase enforcement.** Publish signatures immediately;
   consumers stay `SigLevel = Optional` until a release carrying
   `lisa-keyring` has shipped AND devices have taken it, then flip to
   `Required`. The same overlap discipline as the Zen unbaking
   (ADR-0023): never demand a verification a fielded device cannot yet
   perform.

## The threat model, honestly

CI-held signing defends **tampered downloads and mirrors** — the
integrity of bytes between our CI and a device, wherever they travel.
It does **not** defend a compromise of the GitHub org itself: whoever
can run the publish workflow can sign. Owner-held offline signing
would close that, at the cost of a manual step per publish, on a
project with one maintainer whose OS update channel is already GitHub
releases (sysupdate trusts that exact surface today, ADR-0034
acknowledged it). The marginal risk bought by automation is small; the
rotation path keeps it reversible — a new key reaches devices through
lisa-keyring like any package, so moving to offline signing later
breaks nothing.

## What would change this

- Mirrors or torrent distribution going live promotes key compromise
  from theoretical to the main event: revisit offline signing then.
- A second maintainer makes "the org can sign" a shared-account
  problem; per-person publish rights with a signing service is the
  next shape.
