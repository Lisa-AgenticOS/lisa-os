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
(ADR-0020).

## The lint gates

The `check-*.py` scripts here are the mechanisms behind `just lint`. Each
exists because a defect of its class shipped once and announced itself
nowhere: `check-workflow-quoting.py` (an apostrophe in a workflow comment),
`check-user-units.py` (#161), `check-repart-slots.py` (an A/B slot size
mismatch that only bites on the first update), `check-embedding-model.py`
(#163), `check-tokens.py` (ADR-0038), and `check-app-manifests.py` (#241 —
a manifest installed outside `SYSTEM_MANIFEST_DIR`, which costs an app its
entire agent surface with no error, warning or log line), and
`check-egress-units.py` (#275 — `lisa-inferenced-dbus.service`, the unit
every Assistant prompt goes through, shipped with **no** egress sandbox
while two of its own comments described one).

`check-egress-units.py` is the one that discovers its own population:
it reads the PKGBUILD install lines to find every shipped unit, takes each
unit's `ExecStart` binary, and demands a posture for it — so a second unit
for a known daemon is covered with no edit, and a daemon nobody classified
fails the gate rather than passing it. `tests/e2e/egress-test.sh` takes its
unit list from `--list no-egress` so the tested sandbox and the shipped
sandbox cannot become two lists (which is exactly what they had become).

They share a shape: read the truth from the consumer rather than restating
it (`check-app-manifests.py` reads `SYSTEM_MANIFEST_DIR` out of
`daemons/agentd/src/main.rs`), print one line on success, name the
offending path on failure, and fail loudly when a sweep matched nothing —
a check that quietly checks nothing is the failure mode these are for.

Extend by adding a `check-*.py` and a line in the `lint` recipe. Prove it
the way #241 was proved: run it against the broken tree first and watch it
go red, then fix the tree; a check only ever seen green is a check nobody
has tested.

## The generators

Two scripts here produce committed output and gate it with `--check`, so
source and output cannot disagree on `main`:

- **`build-knowledge.py`** — the OS knowledge pack (`docs/knowledge/`)
  from a curated list of component READMEs (#175, ADR-0040). Consumed by
  `lisa context sync-knowledge` and the lisaos.dev docs build.
- **`build-adr-index.py`** — the index table in `docs/adr/README.md`,
  from each ADR's own `- **Status:**` line. It exists because the
  hand-written page claimed "36 of the 37 records below carry no status
  line" while there were 50 records and all 50 had one, and its
  what-is-built table stopped at ADR-0038, so absence read as "not
  built". Beyond staleness it rejects three things: a status line not in
  the canonical shape, a state outside the vocabulary (`proposed`,
  `accepted`, `accepted, partially executed`, `accepted, not
  implemented`, `superseded by ADR-NNNN`, `superseded in part by
  ADR-NNNN`, `status unverified`), and a supersession naming an ADR that
  does not exist. All four failure modes were checked by breaking the
  tree and watching each one go red.

Regenerate with the script and no flag; `just lint` runs both with
`--check`.

## Backlog (Appendix D)

- `snapshot.sh` — record/advance the pinned snapshot date; advances only
  at channel promotion after a soak (PLAN §6).
- Package signing + hosted repo (M1); until then local repos install
  with `SigLevel = Optional`.
- CI wiring so mkosi (Track I) and the layer (Track L) resolve packages
  from the same snapshot.
