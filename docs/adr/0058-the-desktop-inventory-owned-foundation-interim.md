# ADR-0058 — the desktop inventory: owned, foundation, interim

- **Status:** accepted
- **Date:** 2026-08-06
- **Resolves:** #284 (a fork wearing a stock name), and the class it
  belongs to
- **Extends:** ADR-0048 (Lisa Desktop is a desktop, not a patched GNOME)
  — this is that decision made enumerable
- **Related:** ADR-0038 (fork the Shell, not Mutter), ADR-0039 (the split
  and the package index), ADR-0012 (the Intelligence panel that #284
  would delete), ADR-0021 (the aarch64 lane's stock shell), ADR-0034 §7a
  (install/update may not depend on infrastructure we do not control),
  ADR-0051 (ports are built on change), CLAUDE.md rule 11 and the repo
  mechanics rule *"fork packages replace stock by contract, never by
  name"*
- **Claims:**
  - `path:os/repo-tools/check-desktop-inventory.py` — the gate this ADR decides
  - `symbol:check-desktop-inventory\.py@justfile` — wired into the lint gate CI enforces
  - `symbol:FOUNDATION = \{@os/repo-tools/check-desktop-inventory.py` — the foundation set is code, not prose
  - `symbol:INTERIM = \{@os/repo-tools/check-desktop-inventory.py` — and so is the honest interim
  - `symbol:provides=\("gnome-control-center=\$pkgver"\)@os/packages/lisa-desktop-control-center/PKGBUILD` — Settings replaces stock by contract
  - `symbol:conflicts=\(gnome-control-center\)@os/packages/lisa-desktop-control-center/PKGBUILD` — and carries both halves of it
  - `symbol:conflicts=\("\$_pkgname"\)@os/packages/lisa-desktop-online-accounts/PKGBUILD` — Online Accounts likewise
  - `nomatch:^  gnome-control-center$@os/packages/lisa-desktop-control-center/PKGBUILD` — nothing here is built under the stock name any more
  - `symbol:lisa-desktop-shell@os/mkosi/mkosi.conf.d/x86_64.conf` — the worked example is what the x86_64 image installs

## Context

The reference iMac (v20260805.81) installs **672 packages**. Before this
record, nobody could say which of them are ours by intent, which are
stock by intent, and which are simply along for the ride. That is not an
aesthetic complaint. Every desktop-level surprise found in the first week
of August 2026 has exactly one shape — **something arrives, changes, or
gets outranked without any decision being visible** — and the count is
now four:

1. **A fork wearing a stock name.** `gnome-control-center 50.3-2` and
   `gnome-keybindings 50.3-2` on that device are *our* build. They win
   only by `pkgrel`. `vercmp 50.3-2 50.4-1` is `-1`, Arch shipped 50.4-1
   on 2026-08-04 with 51beta staged behind it, so the next image build
   silently ships stock and the Intelligence panel (ADR-0012)
   disappears — with a green build, a booting session, and nothing in
   any log. That is #284. CLAUDE.md already carries the rule this
   violates, *because the same race already ran once*.
2. **A fork that is declared and absent.** `lisa-desktop-online-accounts`
   is named in the release lane's `Packages=`; the device carries stock
   `gnome-online-accounts 3.58.1-1`. Its ports build is correctly
   refusing to run while the OAuth client secret is a placeholder
   (#276) — so the declaration is a promise nothing kept, and nothing
   said so.
3. **Load-bearing packages that nobody declares.** `mutter`, `gtk4`,
   `libadwaita` and `gjs` — the compositor, the toolkit, the widget
   layer and the interpreter every Lisa surface is written in — reach
   the image *only* because the shell package happens to depend on them.
   This is #45's shape (`libcurl` arrived as an accidental transitive
   dependency and `lisa update` was one upstream reshuffle from being
   unable to download), applied to the four components with the most to
   lose. mkosi.conf has already had to declare `nautilus`,
   `gst-plugins-base` and `curl` after being bitten; these four are the
   same bet, unhedged.
4. **A promise made in prose.** `os/mkosi/mkosi.conf` states that
   *"GNOME's foundation stays and is not up for removal — mutter,
   GTK4/libadwaita, gnome-session, gnome-settings-daemon,
   gsettings-desktop-schemas, the portals."* `gnome-session` is in
   `Packages=`, three lines from that sentence. `gnome-settings-daemon`
   is not. **This ADR's own gate found that**, on its first run, and
   that is the argument for the gate in one line: a foundation defined
   in a comment is a foundation nobody can check.

Against all four stands one component that is safe permanently.
`lisa-desktop-shell` replaces GNOME Shell **by contract**: a different
package *name* carrying `provides=(gnome-shell=…)` and
`conflicts=(gnome-shell)`. Arch can ship gnome-shell 52 tomorrow and
nothing displaces it, because there is no version comparison to lose —
the two packages cannot co-exist and only one of them is in any lane's
`Packages=`. Everything below generalises that one worked example.

## Decision

**Every package in a Lisa Desktop image belongs to a bucket, and five of
the six buckets are somebody's stated decision.**

| bucket | what it is | the rule that binds it |
|---|---|---|
| **ours** | a `lisa-*` package we build that replaces nothing | must be declared by a lane and must arrive |
| **fork** | a `lisa-*` package that replaces a stock one **by contract** | `provides=` **and** `conflicts=` on the stock name; the stock name must not be installed |
| **port** | upstream software packaged under its own upstream name because Arch ships none | must be named, with the reason Arch has none |
| **foundation** | deliberately stock, **never to be forked** | forking it is a rule-11 failure; it may not go missing |
| **interim** | stock because the Lisa equivalent does not exist yet | ADR-0048's honest interim; it may not go missing either |
| **transitive** | a dependency nobody chose directly | **not policed** — see below |

Concretely, as of 2026-08-06:

* **ours** (13): `lisa-inferenced`, `lisa-modeld`, `lisa-cli`,
  `lisa-shell`, `lisa-ime`, `lisa-portal`, `lisa-agentd`,
  `lisa-consentd`, `lisa-contextd`, `lisa-notes`, `lisa-remoted`,
  `lisa-keyring`, `lisa-audio-cs8409`.
* **fork** (4): `lisa-desktop-shell` → `gnome-shell` (built by the
  `lisa-desktop` repo, ADR-0039); `lisa-desktop-control-center` →
  `gnome-control-center`; `lisa-desktop-keybindings` →
  `gnome-keybindings`; `lisa-desktop-online-accounts` →
  `gnome-online-accounts`.
* **port** (4): `llama.cpp`, `whisper.cpp`, `piper`,
  `cyrus-sasl-xoauth2`.
* **foundation** (10): `mutter`, `gtk4`, `libadwaita`, `gjs`,
  `gnome-session`, `gnome-settings-daemon`,
  `gsettings-desktop-schemas`, `xdg-desktop-portal-gnome`,
  `xdg-desktop-portal-gtk`, `fcitx5`.
* **interim** (4): `nautilus` (Files is a README), `gnome-console` (no
  Lisa terminal), `gnome-keyring` (no Lisa session credential store),
  `gdm` (ADR-0035's prompt-first login is unbuilt).

`fcitx5` is in **foundation** rather than interim on purpose:
`fcitx5-lisa` is an *addon it hosts* (PLAN §5.7.3 layer 2), which is the
same extend-never-fork relationship GTK4 has. `gsettings-desktop-schemas`
likewise — `lisa-shell` installs an *override* into it, which only works
while the stock schemas are there.

The difference between **foundation** and **interim** is the direction of
travel, and it is load-bearing: forking a foundation entry is a rule-11
violation the gate refuses; replacing an interim entry — by contract,
under a `lisa-*` name — is the plan. **Moving a name between those two
dicts is an ADR, not an edit.**

### Three properties, not three lists

The gate (`os/repo-tools/check-desktop-inventory.py`) follows
`check-egress-units.py`: discover the population, key on the property,
and make an unclassified Lisa-adjacent package an error.

1. **A package we build is `lisa-*`, or it is a declared port.** Nothing
   else. This is #284 caught at its source: a PKGBUILD with
   `pkgbase=gnome-control-center` producing `gnome-control-center` fails
   before it is ever built, let alone shipped.
2. **A claim on a stock name is both halves or neither.** `provides=`
   without `conflicts=` means pacman co-installs stock beside the fork
   and every shared path gets two owners — the lisa-desktop#7 shape, 94
   paths across three pairs. `conflicts=` without `provides=` means
   every dependency on the stock name becomes unsatisfiable and the fork
   is not a replacement but a removal.
3. **A stock name in the image must be a decision written in a
   `Packages=` list.** Not a comment, not an inference: a line in
   `mkosi.conf`, a lane drop-in, or release.yml's generated
   `50-release.conf`. This is what makes the aarch64 lane's stock
   `gnome-shell` legal (ADR-0021 declares it, and the gate prints
   `STOCK BY DECLARATION` so a deliberate pass never reads like a silent
   one) while the device's `gnome-control-center` is not.

Absence fails in every direction. A fork or `ours` package a lane
declares and the image lacks is a failure; a foundation or interim
package missing from the image is a failure; a manifest that is a
directory, empty, or parses to zero rows is a failure. Every gate
audited on 2026-08-06 was vacuous-by-absence in at least one of those
ways, and this one does not add a fifth.

### What we deliberately do not police

**`transitive` is unchecked, and 638 of the device's 672 packages are
in it.** Hand-classifying them would mean inventing classifications,
which CLAUDE.md rule 10 forbids more strongly than it asks for coverage:
a table asserting that `libxkbcommon` is "foundation" would be a fact
nobody established, and it would teach every later reader that the other
rows are guesses too.

What stands in for that coverage is a **ratchet**. Any name this
inventory *does* claim must be declared by some lane's `Packages=`, or be
listed in `UNDECLARED_DEBT` under a stated ceiling. Today that debt holds
six entries — `mutter`, `gtk4`, `libadwaita`, `gjs`,
`gsettings-desktop-schemas`, `gnome-settings-daemon` — which is the exact
list of load-bearing components currently riding in on somebody else's
dependency graph. Adding a seventh costs a second line raising the
ceiling, and that line reads in the diff as *"the number of load-bearing
packages that reach the image by accident goes from 6 to 7"*, which is
not a sentence anyone lands by accident.

The debt is **named here, not paid here**: `os/mkosi/**` is another
change's territory, and a gate that edits the thing it judges is not a
gate.

## Consequences

* CI is red on #284 until it is fixed, and it is red with the reason
  rather than a symptom. The gate's verdict on today's device manifest
  is six findings: three stock names occupying fork slots
  (`gnome-control-center`, `gnome-keybindings`, `gnome-online-accounts`)
  and three declared packages that never arrived
  (`lisa-desktop-control-center`, `lisa-desktop-online-accounts`,
  `lisa-consentd`).
* **Fixing #284 is not this record's job** (that is task #152). The
  in-tree PKGBUILD has already been renamed and carries the contract; the
  remaining work is a ports build and a release, and until it lands the
  device keeps a Settings app that is one Arch bump from reverting to
  stock.
* The `gnome.desktop` session entry still promises a stock GNOME
  fallback that **cannot exist** — `/usr/bin/gnome-shell` *is*
  `lisa-desktop-shell`, verified on the device. This ADR explains why
  that promise is empty (a contract replacement takes the binary path,
  the D-Bus names and the schemas) but does not police session files;
  that is a separate check on the built root.
* Adding a package to any lane now has a cost: if it is Lisa-adjacent it
  must be built here or recorded in `OUT_OF_TREE`, and if it is stock and
  load-bearing it must be classified or left honestly transitive.
* `check-desktop.sh` keeps its half: the shell *pin* (#273) and the
  shell/mutter ABI series (#277). The two gates read the same
  `/usr/lib/lisa/packages.manifest` and answer different questions —
  "is this the desktop the commit intended" versus "is every component of
  the desktop somebody's decision".

## Alternatives considered

**Classify all 672.** Rejected above: it manufactures facts. The ratchet
gives the same protection where it matters and stays true.

**Query the Arch package database to tell a port from a fork-by-name.**
Rejected. It would make the gate need the network, and a gate that
cannot run offline is a gate people learn to skip — the same reasoning
that keeps `check-desktop.sh` to bash builtins. `PORTS` is therefore a
declared list, and every entry carries the reason Arch has no package by
that name.

**Infer the lane from the manifest instead of requiring `--lane`.**
Rejected, and this is the sharpest trade in the file. A release image and
a nightly image differ only in the Lisa packages, so inferring the lane
from "are there Lisa packages present" means an image that lost *all* of
them reads as a nightly and passes. Requiring the lane, and then checking
the claim against the kernel package the manifest actually carries, is
the version that cannot go quietly green.
