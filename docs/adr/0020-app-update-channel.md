# ADR-0020: app updates decoupled from the OS image

- **Status:** accepted
- **Date:** 2026-07-25

## Context

Every app fix currently rides a full OS release: the GJS surfaces (Assistant,
Ledger, Settings, the overlay backend) live under `/usr/share/lisa/shell` in
the immutable image, so a one-line chat-app fix means a ~1 GB image build, a
reboot, and an A/B slot flip. The user asked the right question (2026-07-25):
should apps have their own update system? Immutable-OS practice (Silverblue,
SteamOS) says yes: base image = OS; apps update independently. PLAN §5.12.1's
endgame is Flatpak + capability manifests (M6); this ADR is the interim
channel that works today, exploiting the fact that the shell apps are
*interpreted* — updating them is copying files.

## Decision

A versioned **apps tree** on the persistent `/var`, preferred over the baked
tree when present:

- **Layout:** `/var/lib/lisa/apps/versions/<ver>/` holds a full copy of the
  shell tree; `/var/lib/lisa/apps/current` is a symlink to one version,
  flipped atomically (symlink+rename). No partial states.
- **Launcher indirection:** `/usr/bin/lisa-app <relpath>` resolves the tree —
  `$LISA_APPS_DIR` → `/var/lib/lisa/apps/current` → `/usr/share/lisa/shell`
  fallback — and execs `gjs -m` on the app entry point. `.desktop` files and
  the D-Bus activation file exec via `lisa-app`, so an updated tree takes
  effect on the next app launch — **no reboot**. (GNOME Shell *extensions*
  load at session start from the baked tree and are out of scope for this
  channel — they keep riding image releases.)
- **`lisa apps` verbs** (CLI, rule 7): `update` (fetch the newest
  `lisa-apps_<ver>.tar.zst` release asset, verify against `SHA256SUMS`,
  unpack, flip), `status` (current/available versions), `rollback` (flip to
  the previous installed version). Reuses the same GitHub Releases channel
  and manifest the OS updates use.
- **CI:** release.yml packs `shell/` into `lisa-apps_<ver>.tar.zst`,
  checksummed in the same `SHA256SUMS` as the image artifacts. One release,
  two update planes.
- **Integrity, honestly:** sha256 via the manifest, same trust level as the
  sysupdate transfer set today (`Verify=no` era); GPG-signed manifests land
  with the M1 signed repo for both planes at once.

## Consequences

- App fixes reach devices in minutes without touching a boot slot; a broken
  app tree is one `lisa apps rollback` away, and deleting
  `/var/lib/lisa/apps` always restores the baked tree (the image remains the
  recovery floor).
- The baked tree and the apps tarball ship from the same commit, so versions
  can only skew forward (an apps tree newer than the image). Apps must keep
  degrading gracefully against older daemons — already the house style
  (fail-soft D-Bus calls everywhere).
- Superseded by the Flatpak lane (M6) when it matures; the `lisa apps`
  interface is deliberately small so the migration is a backend swap.

## Amendment, 2026-07-26 (ADR-0023 phase 1, issue #51)

This channel now carries **binary, per-architecture payloads**, not only
interpreted trees. `lisa apps` gained a channel concept:

- `shell` — the GJS tree, exactly as decided above. State stays at
  `/var/lib/lisa/apps` so trees installed by earlier releases keep
  resolving; asset `lisa-apps_<ver>.tar.zst`.
- `zen` — the Zen browser tree that used to be baked as `/opt/zen`. State
  at `/var/lib/lisa/apps/payloads/zen`; assets
  `lisa-zen_<ver>_<arch>.tar.zst`, published for x86_64 **and** aarch64
  from the same release (a repackage of an upstream binary tarball needs
  no native runner, so one release yields a complete channel).

Three rules follow from a payload being ~360 MiB instead of ~2 MiB:

- **Versions are pruned.** Each channel keeps a bounded number of trees
  (`zen`: 2, `shell`: 3), never deleting the one `current` points at.
  Unbounded history was harmless for the shell tree and is not for a
  partition that also holds the model store.
- **Downloads stream.** The payload is hashed straight to disk instead of
  being read into memory first.
- **Some channels auto-sync.** `lisa apps sync` installs only channels
  with **no baked fallback in the image** and no tree yet — today just
  `zen`. The shell tree is excluded: the image still carries it, so
  auto-pulling would skew the tree ahead of the image for nothing. Sync
  never changes the version of a payload that is already installed;
  moving versions stays a deliberate `lisa apps update`.

`lisa apps update` and `rollback` take an optional channel name and
default to all channels. Verbs, layout, atomic flip, and the SHA256SUMS
trust level are unchanged. One consequence of the ADR-0020 recovery floor
does change: for `zen` there is no baked tree to fall back to after the
overlap release, so `rollback` past the oldest installed version leaves
the payload absent and says so, and the launcher's message — not the
image — is the floor.
