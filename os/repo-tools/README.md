# os/repo-tools — pinned snapshot mirror + custom repo

Spec: PLAN §3 ("packaging economics"), §6. We control when the Arch base
moves, like SteamOS's `holo` repo: the image and layer both build
against a **pinned snapshot** of Arch (snapshots served by the Arch
Linux Archive, archive.archlinux.org), plus our own small signed `[lisa]`
repo (~100–200 packages).

## Today

`build-packages.sh [outdir]` — builds the `[lisa]` repo from the current
git HEAD via makepkg + repo-add (run on Arch, host or container, as an
unprivileged user with base-devel + rust). The output directory works as
`Server = file:///path` for `os/layer/install.sh`; the container e2e
(`tests/e2e/layer-test.sh`) exercises the full loop.

`build-apps-payload.sh <stage-dir> [tarball]` — stages the interpreted
app tree (ADR-0020, ADR-0047): the GJS shell surfaces plus Mail, Surfer
and Preview, minus test material. **Two consumers, one list.** The
`lisa-shell` package stages `/usr/share/lisa/shell` with it and
`release.yml` packs `lisa-apps_<ver>.tar.zst` from it, so the copy the
image bakes and the copy `lisa apps update` installs are the same tree
with the same relative layout — `lisa-app mail/lisa-mail.js` means one
thing. Issue #239 is what two lists cost: the release tarball packed
`shell/` alone for four months, so no fix to Mail, Surfer or Preview
could reach a device without a full image update.

Extend it by editing the two arrays at the top; the tree's contract with
the launchers is checked by `cli/lisa/tests/apps_payload.rs`, which
reads every `.desktop`/D-Bus service file the package installs and
asserts the entry point it execs exists in the staged tree.

Limits: it stages files only. `.desktop` entries, D-Bus service files and
GSettings schemas are installed to `/usr` by the package, so a payload
can update an app's *code* but cannot add a new app to the launcher, and
GNOME Shell extensions still load from the baked tree at session start
(ADR-0020). `build-zen-payload.sh <arch> <version> [outdir]` builds the
per-arch Zen payload for the same channel.

## Backlog (Appendix D)

- `snapshot.sh` — record/advance the pinned snapshot date; advances only
  at channel promotion after a soak (PLAN §6).
- Package signing + hosted repo (M1); until then local repos install
  with `SigLevel = Optional`.
- CI wiring so mkosi (Track I) and the layer (Track L) resolve packages
  from the same snapshot.
