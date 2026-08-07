# Lisa OS — packages, apps & forks

The living registry of what Lisa *ships*: first-party apps, forked/patched
upstream, and the shell surfaces — plus how we track and repo them. The
daemons and SDK libs are in the CLAUDE.md/PLAN §5 component map; this doc
is the **apps + forks** view and the **repo/distribution strategy**.
**Updated: 2026-07-23.**

## Repo strategy (short version)

We're a **monorepo** (ADR-0006: staged extraction). Everything lives in
one tree with per-job CI path filters, so changes stay atomic while the
substrate still churns. We **extract** a component to its own repo
(`Lisa-AgenticOS/<name>`) only when a trigger fires:

- external contributors / independent release cadence,
- it ships as its own Flatpak / app-catalog entry,
- or it reaches a stable public API.

Until then, adding an app or a fork means adding a row here + a package,
not a new repo.

## First-party apps

Lisa-native apps are **GJS + GTK4/Adwaita** (ADR-0047). Interpreted
source, no build step, no toolchain: an app is a directory you can
`cp -a`. `lisa dev check` (ADR-0050) is the authority on whether one is
valid. Status is honest.

| App | Path | Tech | Does | Status |
|---|---|---|---|---|
| Mail | `apps/mail` | GJS/libadwaita | Maildir client with an MCP surface | live (packaged) |
| Surfer | `apps/surfer` | GJS/libadwaita | WebKit browser the model can drive | live (packaged) |
| Preview | `apps/preview` | GJS/libadwaita | document/image preview + its MCP tools | live (packaged) |
| Notes | `apps/notes` | GJS/libadwaita | AI-native notes (reference app) | seed (M6) |
| Recorder | `apps/recorder` | GJS/libadwaita | audio capture + on-device transcribe | seed (M6) |
| Forge / LisaCode | *(no window yet — ADR-0061)* | forge-harness | writes + installs apps locally (§5.12.1) | the loop works via `lisa forge`; the window is future work |
| Ledger | `shell/ledger-app` | GJS/libadwaita | the append-only audit log viewer | live (packaged) |
| Lisa Settings | `shell/settings` | GJS/libadwaita | AI settings: local models + providers | **not shipped** — merged into GNOME Settings as the Intelligence panel (ADR-0012); source kept as reference + tests |

## Shell surfaces (GNOME, GJS)

Not apps and not forks — GNOME Shell **extensions** + helper surfaces,
shipped in the `lisa-shell` package. Extensions, not forks, because GNOME
supports them as a stable extension point.

| Surface | Path | What |
|---|---|---|
| Assistant overlay | `shell/overlay-extension` | Spotlight-style summon + backend |
| Semantic launcher | `shell/launcher` | type-to-find apps/actions/answers |
| Ledger app | `shell/ledger-app` | audit-log app (also in the apps table) |
| Settings | `shell/settings` | AI settings app (also in the apps table) |

## Forked / patched upstream

What we don't write from scratch but must own a delta on. **Every fork
needs an ADR** and a pinned upstream version. Prefer *thin patches* over
hard forks: track the delta, not a diverged tree.

| Package | Upstream | Pinned | Why (ADR) | Delta | Repo |
|---|---|---|---|---|---|
| `lisa-desktop-control-center` | gnome-control-center | 50.3 | no plugin API for a sidebar panel (ADR-0012) | panel dir + 2 anchored edits | in-tree `os/packages/` |
| Terminal integration | GNOME Console/VTE | TBD | `lisa` CLI presence | integration | `apps/terminal-integration` |

**No app patch sets (ADR-0048).** This table used to carry rows for Files,
Mail and Photos. Each was a scaffold directory holding one README that
said "not started", and no patch was ever written in any of them. We write
the apps instead: `apps/mail` shipped, `apps/files` and `apps/photos` are
not-started Lisa apps rather than planned patch sets. Where a Lisa app
does not exist yet, the image ships the stock GNOME app **unpatched**.

`lisa-desktop-control-center` is the one remaining patch set, and ADR-0048
§3 puts it on a path to retirement in favour of `shell/settings` — a
direction, with conditions, not something done today.

Forks stay **thin, maintained patches in-tree** (build upstream at a
pinned version, drop in our files, apply guarded anchored edits — see
`lisa-desktop-control-center`). We re-pin on a GNOME bump; a moved anchor
fails the build loudly. A fork only earns its own repo if the patch grows
past "thin."

## Bundled third-party (runtimes & apps)

Not ours, not forked — upstream software shipped in the image so the OS is
useful out of the box. Official Arch packages go straight in `mkosi.conf`;
AUR-only ones get a thin PKGBUILD in `os/packages/` built into the release
repo (like the lisa packages).

| Package | Why | Source | Where |
|---|---|---|---|
| `llama.cpp` | local inference engine (llama-server) for inferenced | from source (b10093, MIT) — AUR-only | `os/packages/llama.cpp` → release repo |
| `dart` | `dart analyze`/`dart test` for pubspec projects the forge loop may sit over; the forge lane itself uses `lisa dev check` and needs no toolchain — issue #327 tracks re-scoping this | Arch `extra` | `mkosi.conf` Packages |
| **Flutter** | nothing — the lane was removed 2026-08-07 (ADR-0047 amendment) | **not bundled, not needed** | — |

**Flutter is not in the image, and no longer on the app road** (ADR-0047
parked the lane; §3 closed #37 won't-do; the 2026-08-07 amendment removed
it). It was excluded for size — ~1.5 GiB carried by every A/B update —
and is now excluded for a better reason: a Lisa app is interpreted source
that needs no build toolchain at all. `lisa forge --setup`, which fetched
the pinned SDK for the parked lane, no longer exists. The lisa-cli
package installs no `lisa.sdk` payload and declares no Flutter
optdepends (#246).

**The browser is no longer bundled third-party software.** Zen — a
repackaged upstream tarball, 363 MiB of root payload and 726 MiB across
the A/B pair — was the interim browser. It was first moved out of the
image onto the apps channel (ADR-0023 phase 1, issue #51) and then
retired entirely on 2026-08-05, because the browser we ship is now
**Surfer** (ADR-0037): a first-party GJS/WebKit app in `apps/surfer`,
installed by the `lisa` package as `app.lisaos.Surfer` and listed in the
first-party table above. Nothing repackages an upstream browser tarball
any more, so there is no `zen-browser` port, no `lisa-zen_*` channel
artifact, and no `zen` payload channel.

## SDK / libraries (pointers)

`libs/`: `liblisa` (+ gtk/qt), `forge-harness` (the LisaCode loop),
`harness-core`, `lisa-guard`, `lisa-ledger`, `mcp-bus`. `lisa.sdk` and
`lisa_flutter` were the Flutter lane and were **deleted** on 2026-08-06
(ADR-0047 chose GJS; two kits one underscore apart was a trap). The
name `lisa.sdk` is reserved for the shared GJS/GTK4 library ADR-0047 §6
asks for, **which is not written yet — the directory does not exist**. Details in the
PLAN §5 component map.

## How elementary OS does it (why we differ, for now)

elementary is the closest reference — restrained, humane, its own
identity. Their model:

- **Per-app repos** under one org (`elementary/files`, `/terminal`,
  `/mail`, `/music`, `/tasks`, `/calculator`, `/code`, …). Dozens of small
  repos.
- **A widget/SDK library** — `granite` (their GTK toolkit) — the role
  the shared GJS/GTK4 library is meant to play for us (ADR-0047 §6,
  unwritten; today each app carries its own copy, which is how #218 had
  to be fixed three times).
- **Shell as separate repos** — `gala` (compositor on libmutter),
  `wingpanel` (top bar), `greeter`, and **`switchboard`** — their Settings,
  which is **plug-based**: each panel is its own repo (`switchboard-plug-*`)
  and third parties can add plugs. (We had to *fork* gnome-control-center
  precisely because it is *not* plug-based — ADR-0012. If we ever adopt a
  plug-able settings shell, that fork goes away.)
- **They avoid hard forks** — build on libmutter / GTK, extend via
  Granite + plugs, ship a `stylesheet` (GTK theme) and `icons`, not forked
  toolkits.
- **Distribution** — AppCenter (Flatpak, pay-what-you-want) + their apt
  repo; an `os` meta-repo assembles the ISO.

**Our stance now:** monorepo (ADR-0006) beats dozens of repos while the
whole substrate is in flux — atomic cross-cutting changes, one CI, one
review. We mirror the *good* elementary ideas without the repo sprawl:
a shared GJS library ≈ Granite; shell surfaces ≈ their shell repos; a
thin g-c-c fork instead of a plug (until a plug-able shell exists). At
**M6 / public alpha**, when apps are user-installable (the LisaCode/Forge
vision: "everyone can have their own apps") and there's a community, we
extract mature apps to their own repos + a Lisa app catalog —
elementary-style.

## Adding something

- **New first-party app:** create `apps/<name>` as GJS + GTK4/Adwaita
  (ADR-0047 — there is no second toolkit to choose between), check it
  with `lisa dev check`, add a row above, and package it by extending
  `lisa-shell`. Read `docs/ANATOMY-OF-AN-APP.md` first: it is derived
  from the apps that exist and its §7 is the five defects that already
  shipped. No new repo until an extraction trigger fires.
- **New fork/patch:** write an ADR (why upstream can't do it unpatched),
  pin the upstream version, keep the patch thin + guarded, add a row to
  *Forked / patched upstream*. Never a silent divergence.
