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

`check-desktop-inventory.py` gained **R8** on 2026-08-06, and it is
worth reading as a pattern rather than a rule. R2 already asked whether
a fork's contract was *coherent* — provides and conflicts, both halves
or neither. R8 asks the different question nobody had asked: does
anything **act** on it? `replaces=` was missing on all three forks, and
without it pacman never offers the swap, because a machine holding
stock has nothing pulling a differently-named package in. So the
2026-08-05 rename read as complete in the tree, in `ports.lock` and in
the ADR while having happened on zero devices (#284). Coherent and
inert are not the same property, and only one of them was checked.

`check-egress-units.py` is the one that discovers its own population:
it *interprets* the PKGBUILD install lines to find every shipped unit,
takes each unit's `ExecStart` binary, and demands a posture for it — so a
second unit for a known daemon is covered with no edit, and a daemon nobody
classified fails the gate rather than passing it. `tests/e2e/egress-test.sh`
takes its unit list from `--list no-egress` and its drop-in list from
`--dropins` so the tested sandbox and the shipped sandbox cannot become two
lists (which is exactly what they had become).

"Interprets" is load-bearing, and #291 is why. The first version matched
literal whitespace tokens, so a source in a shell variable, from a `for`
loop, or behind a glob resolved to nothing and was **dropped in silence** —
sixteen real install lines, over the unmodified tree. It now walks the
installer with an environment, expands loops and globs, and **fails on any
install into a systemd directory it cannot resolve**; `--installs` prints
the whole resolution table so "there is nothing there" can be told apart
from "I could not parse that". A second, blunter net catches whatever the
walker cannot read: every systemd fragment in `os/packages/**` must be
installed by some line, so a unit added by an unmodelled idiom fails as an
orphan rather than shipping unseen. Drop-ins are merged into the unit
before it is judged, and directives are read with systemd's own last-wins /
empty-resets semantics (#292).

    python3 os/repo-tools/check-egress-units.py --installs   # the table
    python3 os/repo-tools/check-egress-units.py --explain    # the postures

It reads **D-Bus activation records** from the same install lines (#294).
`dbus-daemon` starts a service itself when the record carries only
`Exec=` — no unit, therefore no sandbox, and no row in this gate at all,
because the population comes from systemd install lines and there is no
unit to install. That is the same "`[Install]` lies by omission" problem
arriving through the other door, and it is how `dev.lisaos.Overlay1` and
`dev.lisaos.Voice1` — the backend that historically hosted the model —
ran unconfined and unclassified. A record whose `Exec=` names a Lisa
binary must name a `SystemdService=`, and that unit must be one
discovery actually installs; a record for somebody else's binary is not
rule 5's business and is left alone.

Its three debt lists (`EXEMPT`, `USER_SCOPE_INET_DEBT`,
`DBUS_UNSANDBOXED_DEBT`) each remove something from a check, so all are
**ratcheted** against `DEBT_CEILING` (#293): adding an entry fails unless
the same commit also raises the ceiling, and *removing* one fails unless
it lowers it. That does not make an entry impossible — nothing in a gate
can — but it removes the free, green, one-line version and puts the
number in the diff.

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
