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

## Backlog (Appendix D)

- `snapshot.sh` — record/advance the pinned snapshot date; advances only
  at channel promotion after a soak (PLAN §6).
- Package signing + hosted repo (M1); until then local repos install
  with `SigLevel = Optional`.
- CI wiring so mkosi (Track I) and the layer (Track L) resolve packages
  from the same snapshot.
