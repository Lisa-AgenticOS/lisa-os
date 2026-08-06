# ADR-0057 — the monorepo owns the shell surfaces until step 6 actually happens

- **Status:** accepted
- **Date:** 2026-08-06
- **Resolves:** lisa-desktop#7
- **Amends:** ADR-0039 (the split and the package index) — not its
  direction, its schedule and its ordering
- **Related:** ADR-0048 (Lisa Desktop is a desktop), ADR-0056 (`lisa_ui`
  is the dialect) §"One package, built from the monorepo"
- **Claims:**
  - `symbol:conflicts=\(lisa-desktop lisa-apps\)@os/packages/lisa/PKGBUILD` — `lisa-shell` declares the conflict this ADR decided
  - `symbol:conflicts=\(lisa-desktop-ime\)@os/packages/lisa/PKGBUILD` — and `lisa-ime` the IME half
  - `path:os/repo-tools/check-package-paths.py` — the gate that reads the published index rather than this tree

## Context

ADR-0039 extracted `lisa-desktop`, `lisa-apps` and `lisa-packages` on
2026-08-02 with history, and left step 6 — deleting `shell/` and `apps/`
from the monorepo — undone. Its own failure clause therefore triggered:
two trees hold the same source, and CLAUDE.md carries a standing warning
to *"know which tree ships the thing you are changing."*

lisa-desktop#7 reported that the honest answer is "both do", and named
eight colliding paths between `lisa-desktop` and `lisa-shell`.

Measured against the published `[lisa]` index on 2026-08-06 — the
`lisa.files` database, which is the exact per-package file list of what
actually ships — the collision was larger and had a shape nobody had
described:

| pair | colliding paths |
|---|---|
| `lisa-desktop` ↔ `lisa-shell` | 51 |
| `lisa-apps` ↔ `lisa-shell` | 39 |
| `lisa-desktop-ime` ↔ `lisa-ime` | 4 |

**94 paths, three pairs, and not one of the five packages declared
`conflicts`, `provides` or `replaces`.** Two of the three pairs had never
been reported; they came out of enumerating the index rather than reading
the issue. The issue's other claim — that the two ship different shapes
at `/usr/share/gnome-shell/extensions`, real directories against symlinks
— is **not true at HEAD**: both ship identical symlinks into
`/usr/share/lisa/shell`, verified by listing both `.pkg.tar.zst` files.

Three further facts decide this ADR.

1. **The collision is latent.** `/usr/lib/lisa/packages.manifest` on the
   reference iMac lists `lisa-shell`, `lisa-ime` and
   `lisa-desktop-shell`. It does **not** list `lisa-desktop`,
   `lisa-desktop-ime` or `lisa-apps`. The image installs the monorepo's
   surfaces and the fork; the extracted surface packages are built,
   signed and published, and installed nowhere.
2. **`lisa-desktop` is a strict subset of `lisa-shell`.** Set difference
   over the index: `lisa-desktop` has **zero** files that `lisa-shell`
   does not also ship. It is not a different packaging of the surfaces;
   it is an older copy of them.
3. **`lisa-desktop` and `lisa-apps` together are 40 files short**, and
   the missing files are not trivia. They include
   `launcher/schemas/org.gnome.shell.extensions.lisa-launcher.gschema.xml`
   (#255 — an extension whose `getSettings()` throws does not degrade, it
   fails to enable at all), `assistant/app.lisaos.Assistant.service`
   (#210 — the Spotlight hand-off's D-Bus activation),
   `desktop/lib/stateicon.js` and `badges.js` (#190). Worse,
   `lisa-apps` *regresses two closed issues*: it ships the dead
   `org.gnome.NautilusPreviewer2.service` (Nautilus dials the
   versionless name and ping-gates it, so Space is dropped silently) and
   installs Preview's manifest to `/usr/share/lisa/apps/`, which is
   **#241 verbatim** — `SYSTEM_MANIFEST_DIR` in
   `daemons/agentd/src/main.rs` is `/usr/share/lisa/manifests` and
   nothing else is read, so Preview's tools reach no model.

## Decision

**The monorepo owns the shell surfaces, the app tree and the IME addon.
`lisa-desktop` narrows, in practice, to the GNOME Shell fork
(`lisa-desktop-shell`).** That is option (b) of the three the issue
offered.

This reverses ADR-0039's *stated* intent, which was that lisa-desktop
owns the surfaces and the IME. It does not reverse its direction. The
extracted repos are the right long-term home; they are not the current
one, because ownership follows maintenance and maintenance has stayed
here. A package built from a tree nobody edits is not an owner — it is a
stale copy with a signature on it, which is strictly more dangerous than
no package at all.

### Why not (a) — finish step 6 now

Because (a) is not reachable by a packaging change, and fact 3 is the
proof. Swapping `lisa-shell` for `lisa-desktop` + `lisa-apps` today would
remove 40 files from a device and reintroduce #241 and the
NautilusPreviewer name. Step 6 is a **source migration**: the 40-file gap
closes in `shell/` and `apps/`, in the extracted repos, before any
package changes hands. The gap is now measured, which is the one thing
that was missing to schedule it (lisa-os#171 step 4).

### Why not (c) — coexist with mutual conflicts

`conflicts=` is the *mechanism* this ADR uses, but it is not the
*decision*. Two packages that conflict are alternatives a user chooses
between; these are one maintained tree and one stale copy of it, and
nobody should choose the second. The conflict makes the mistake loud
while the migration is outstanding; it does not bless the duplication.

### Not `replaces=`

`replaces=` would make the next `pacman -Syu` perform the swap
unattended, which is exactly the 40-file regression above. The same
argument the `lisa-desktop-shell` PKGBUILD already makes against
`replaces` for the Shell fork applies here with more force, because here
the replacement is known to be incomplete.

## Consequences

- `lisa-shell` declares `conflicts=(lisa-desktop lisa-apps)`;
  `lisa-ime` declares `conflicts=(lisa-desktop-ime)`; `lisa-desktop` and
  `lisa-desktop-ime` declare the reverse. A file owned by two packages in
  the signed index becomes an install-time refusal rather than a race.
- **A device is unaffected.** `conflicts` binds when both packages are in
  one transaction; no device has both, so the next update is an ordinary
  update. Installing `lisa-desktop` on a Lisa device now prompts to
  remove `lisa-shell` in a single visible transaction instead of failing
  halfway through with "exists in filesystem".
- `os/repo-tools/check-package-paths.py` fails when two packages in the
  index own a path without a declared conflict. It reads the **published
  index**, because after ADR-0039 no single tree contains the whole
  package set and a check that reads only this repo cannot see this
  class of defect.
- **`lisa-desktop`, `lisa-desktop-ime` and `lisa-apps` should stop being
  published** until they are the maintained source. That is a
  `lisa-packages` change and is not made here; until it is, the conflicts
  above are what keeps them off a device.
- ADR-0056 is unaffected and slightly strengthened. Its argument — the
  shared library is one package built from the monorepo because three
  consumers share it — is the same argument this ADR reaches from the
  other end: shared, maintained code has one home, and today that home is
  here. ADR-0056 §"What this ADR does not decide" left "whether the shell
  surfaces in `lisa-desktop` consume the same package or a subset" to
  fall out of resolving #7. It falls out as: they consume it from the
  monorepo, like everything else, until step 6 moves the source.

## What this ADR does not decide

- When step 6 happens. It decides only that the 40-file gap closes first,
  and that the gap is now enumerated rather than assumed.
- Whether `lisa-desktop` should be deleted from the index or merely left
  unpublished. Deleting a signed artifact users may have pinned is its
  own decision, and no user has it installed.
