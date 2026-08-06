# ADR-0022: A user-survivable rescue boot path

- **Status:** accepted, partially executed — phases 1–3 are implemented and
  proven by execution (phase 1 self-repair and phase 3's shell on an
  unbootable machine are green in ab-recovery; phase 2's resolver refuses a
  half-written slot on its GPT type in ab-interrupted-transfer). Still
  missing: the boot ENTRY — the resolver works, nothing offers it in the
  menu (#23).
- Date: 2026-07-25
- Relates: issue #23, issue #20 (the incident), ADR-0018 (PARTLABEL
  mounts), PLAN §3 (Track I), §6 (updates), M7 (installer/OOBE)
- Amended by [ADR-0028](0028-initrd-overlay-mechanism.md): phase 2's root
  resolver was placed in `os/mkosi/mkosi.initrd/mkosi.extra/`, which mkosi
  reads from nowhere, so it never reached the initrd — that is why its CI
  boot waited 90 s for a device nothing created and logged nothing. It now
  ships in `os/mkosi/initrd-overlay/` and the nightly asserts it inside the
  built UKI. The rescue *entry* is still not installed, for the unrelated
  sd-boot default-selection reason in issue #23.
- **Claims:**
  - `path:os/mkosi/initrd-overlay/usr/lib/lisa/newest-good-root.sh` — the phase-3 resolver
  - `symbol:ab-recovery:@.github/workflows/nightly.yml` — proven by execution, not asserted
  - `path:os/mkosi/make-rescue-uki.sh` — the entry that exists and is not installed (#23)

## Context

The #20 incident left the field iMac in the worst boot state the A/B
design can produce: the booted slot's filesystem erased under it, the
ESP holding a dangling boot entry for the erased slot plus an entry for
a broken slot, and one complete, healthy root on disk **with no entry
pointing at it**. Every automatic layer then failed the user:

- systemd-boot happily showed entries whose `root=` no longer resolved.
- The initrd timed out and dropped to emergency mode — where **sulogin
  refused entry because root is locked** ("the root account is locked").
  For a normal user this is a brick with extra steps.
- The actual rescue (hold Space → `e` on an entry → hand-edit
  `root=PARTLABEL=…` → boot) is expert-only knowledge.

Boot-counting protects against a *bad kernel/userspace* (tries decrement,
sd-boot falls back to the previous entry). It does **not** protect
against this class: entries whose backing partition is gone, or a slot
erased after its entry was already marked good.

## Decision

Three mechanisms, smallest-first, each independently useful:

### 1. Emergency mode must admit the user (rd.sulogin-force + wall text)

Ship `rd.systemd.unit_success`—no: concretely, add to the image:

- `systemd.setenv=SYSTEMD_SULOGIN_FORCE=1` is rejected (it allows a
  passwordless root shell — unacceptable on a login screen). Instead the
  initrd's emergency service gets a drop-in replacing `sulogin` with
  `sulogin --force` **only when the root account is locked AND the
  rescue entry booted** (see #2) — a maintenance shell you explicitly
  chose from the boot menu is a different trust context than one a
  random boot failure dropped you into.
- Emergency mode in a *normal* boot keeps locked-root behavior but the
  console message is replaced with Lisa instructions: "hold Space at
  power-on and choose Lisa Rescue" (plus the QR to the recovery doc).
  The message lives in a drop-in `ExecStartPre=` echo — no code.

### 2. A pinned rescue entry: `lisa-rescue.efi` (the core mechanism)

The ESP permanently carries one **rescue UKI** outside the A/B rotation:

- Installed at image build and **refreshed only after a slot proves
  itself**: the same boot-success path that marks a slot good (the
  existing `lisa-boot-report.service` moment) copies the *currently
  booted, known-good* UKI to `EFI/Linux/lisa-rescue.efi` when its
  version is newer than the rescue copy. The rescue UKI is therefore
  always a version that booted this machine at least once.
- Its cmdline differs from the A/B entries in one way: instead of a
  pinned `root=PARTLABEL=root_<ver>`, it boots with
  `root=lisa-newest-good` — resolved by a tiny initrd udev/generator
  shipped in ADR-0021-style overlay: enumerate `root_*` partitions on
  the loader disk (ADR-0018's #16 scoping applies), pick the one whose
  superblock mounts read-only successfully with the **highest version
  suffix**, and link it. That is exactly the manual rescue the iMac
  needed, automated: "boot whatever complete root actually exists."
- sd-boot title: **"Lisa Rescue"**, sorted last, never default,
  `@saved` untouched. It is present in the menu every boot — the user
  instruction is always just "hold Space, pick Lisa Rescue".

### 3. Boot-report-driven self-repair (builds on what exists)

`lisa-boot-report.service` already dumps boot state to the ESP. Extend
it forward-looking: on every successful boot it also
- deletes boot entries whose `root=PARTLABEL=` matches **no existing
  partition** (the dangling-.22 case cleans itself), and
- re-creates a missing entry for any complete root partition that has a
  matching UKI on the ESP (the "healthy v26 with no entry" case heals
  without a keyboard).

Both actions are logged to the Ledger (kind `boot.repair`).

## What was rejected

- **`SYSTEMD_SULOGIN_FORCE=1` globally** — passwordless root on any
  boot failure; no.
- **Setting a root password at install** — expands attack surface and
  contradicts the passwordless-admin-user model; the provisioned user's
  credentials can't drive sulogin without PAM surgery.
- **A second full recovery OS partition** (Android-style) — costs ~10 GB
  and a second update channel; the rescue UKI + newest-good-root covers
  the realistic failure set at zero partition cost.
- **GRUB-style scripted fallback** — we are not adding a second boot
  loader; sd-boot's simplicity is load-bearing (PLAN §3).

## Phase 3: the console the owner is dropped at (issue #23, 2026-07-31)

Everything above is about reaching a *working* system. This is about the
case where none exists and systemd drops to `emergency.service` — which
runs `sulogin`, which refuses:

```
You are in emergency mode... Cannot open access to console,
the root account is locked.
```

Verified on the reference iMac: `passwd -S root` reports `L`, both
`emergency.service` and `rescue.service` run `systemd-sulogin-shell`,
and no drop-in exists. The rescue path ends at a refusal.

**Decision: set `SYSTEMD_SULOGIN_FORCE=1` on both units**, shipped as a
drop-in in the `lisa` package. `systemd-sulogin-shell` reads it from the
environment and from the kernel command line; the drop-in covers the
first, and the second remains available to somebody who has already
opened the boot menu.

Both units, not just `emergency`: a failed boot lands in `emergency`,
`systemctl rescue` lands in `rescue`, and a recovery path that works
from only one of them is one a person finds by luck.

### Why this is not a security regression

The root filesystem is not encrypted. Anyone at the machine can already
read and rewrite every byte with a USB stick — which is precisely how
the device was recovered after field incident #20. Refusing them a
shell protects nothing and converts a recoverable fault into a brick.

It is also the wrong shape of guardrail. A guardrail sits between the
model and the machine, never between a person and their own machine
(CLAUDE.md 6a, ADR-0030). Emergency mode is reached by a human at a
console *after their computer failed to boot*: the last place to be
enforcing policy against them.

### What would change this

TPM-backed disk encryption (PLAN §3, `systemd-cryptenroll`) inverts the
reasoning — a forced rescue shell on an unlocked rootfs becomes a way
*past* the encryption rather than a way around a locked account. This
drop-in is to be reconsidered as part of that work rather than carried
forward by inertia.

## Acceptance (per issue #23)

A nightly `ab-recovery` job that reconstructs the #20 disk state —
dangling UKI entry, one broken slot, one healthy slot without an entry —
then (a) boots the Lisa Rescue entry and asserts it lands in the healthy
root, and (b) boots normally and asserts the self-repair pass removed
the dangling entry and restored the missing one, with `boot.repair`
ledger entries present.

Phase 3 adds (c): with **both** slots broken, the machine reaches a
usable shell rather than the locked-account refusal — the state that
required a USB reinstall in field incident #2 (#45). Asserting the
drop-in is installed is not the same claim and is not enough; the
assertion has to be that a shell appears.

## Consequences

- The user instruction for *any* unbootable Lisa system becomes one
  sentence: **hold Space at power-on, choose Lisa Rescue.**
- The rescue UKI ages (it's the last *proven* version, not the newest
  staged one) — by design: rescue must prefer proven over fresh.
- ESP needs ~120 MB headroom for one extra UKI; the 1 GB ESP has it.
- The self-repair pass writes to the ESP on boot; it reuses the
  boot-report service's existing privilege, no new surface.
