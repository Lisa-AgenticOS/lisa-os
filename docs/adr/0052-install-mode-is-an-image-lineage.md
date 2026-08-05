# ADR-0052 — Install mode (server/desktop) is an image lineage chosen at install, not a package toggle

- **Status:** accepted, not implemented — no server image flavor exists;
  this records the owner's direction of 2026-08-05 and what makes it
  cheap now.
- **Date:** 2026-08-05

## Context

The full rename (2026-08-05) made the desktop a *contractual layer*:
everything desktop-flavored is a `lisa-desktop-*` package that replaces
its stock GNOME counterpart via provides/conflicts. For the first time,
"the OS without the desktop" is a subtraction the package graph can
express, not an archaeology project.

Meanwhile ADR-0031 (make and serve) is accepted with zero
implementation (#158), and its natural substrate is exactly a headless
Lisa: daemons, CLI, guard, Ledger, models, update channel — no
compositor, no GNOME platform.

The owner asked: "when we install, based on install mode
(server/desktop) we decide, right?" Yes — and this ADR pins down what
"deciding" means in an A/B image OS, because it is not what it means
on a mutable distro.

## Decision

**The mode is which image lineage a disk is installed with and updates
against — decided once, at install (later: in the M7 OOBE), never by
adding or removing packages on a running system.**

1. Two flavors from one build: `lisa` (desktop, today's image,
   unchanged and default) and `lisa-server` (the same image minus the
   `lisa-desktop-*` family and the GNOME platform: no gdm,
   gnome-session, mutter, no GTK apps). mkosi profiles over one config
   tree — never two config trees.
2. **The update channel is part of the mode.** A server install's
   sysupdate transfers name `lisa-server_@v` artifacts; a desktop
   install's name `lisa_@v`. A server machine cannot update into a
   desktop image or vice versa, by construction — the channel does not
   know the other lineage exists. Switching mode is a reinstall (or an
   explicit future `lisa install --switch`, which is a re-image, and
   must say so).
3. `lisa install <disk> --server|--desktop` (default desktop) makes
   the choice; the M7 guided OOBE asks the same question a nicer way.
   One decision point, stated in the tool the person already uses.
4. The server flavor is ADR-0031's substrate. Nothing in it may need a
   display: acceptance for the flavor is boot → daemons up →
   `lisa ask` answers over ssh, on an image that contains no
   compositor at all.

## What this is not

- Not a package toggle: `pacman -R` on an A/B image OS is a lie the
  next update reverts. The image is the unit; the flavor is the
  lineage.
- Not a second maintenance surface: profiles share one mkosi tree, one
  package set definition, one release pipeline. The server image is a
  strict subset — if it ever needs a package the desktop lacks, that
  package was misfiled.
- Not free build time: a second image roughly doubles the image-build
  half of the release. Sequencing options (build server weekly or
  on-dispatch rather than per-release) are an implementation decision
  for the issue, made against measured build times — not here.

## Consequences

- The desktop/OS seam gets a consumer, which keeps it honest: any
  `lisa-desktop-*` dependency that leaks into a substrate package
  breaks the server build loudly.
- Release artifacts gain a lineage dimension (`lisa-server_@v.root.xz`
  etc.); SHA256SUMS, the installer and the docs must present the two
  lineages without letting a human grab the wrong one.
- ADR-0031's "sequenced after the injection suite" gate still applies
  to *serving*; the server image itself carries no new capability and
  need not wait for it.
