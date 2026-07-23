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

Lisa-native apps. Flutter apps use `libs/lisa_ui` (our design system, no
Material/Cupertino) + `libs/lisa_flutter` (SDK). Status is honest.

| App | Path | Tech | Does | Status |
|---|---|---|---|---|
| Notes | `apps/notes` | Flutter + lisa_ui | AI-native notes (reference app) | seed (M6) |
| Recorder | `apps/recorder` | Flutter + lisa_ui | audio capture + on-device transcribe | seed (M6) |
| Forge / LisaCode | `forge/app` | Flutter + forge-harness | writes + installs Flutter apps locally (§5.12.1) | seed; the loop works via `lisa forge` |
| Ledger | `shell/ledger-app` | GJS/libadwaita | the append-only audit log viewer | live (packaged) |
| Lisa Settings | `shell/settings` | GJS/libadwaita | AI settings: local models + providers | live (packaged) |

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
| `gnome-control-center-lisa` | gnome-control-center | 50.3 | no plugin API for a sidebar panel (ADR-0012) | panel dir + 2 anchored edits | in-tree `os/packages/` |
| Files patches | GNOME Files (nautilus) | TBD | Lisa context/agent hooks | patch-set | `apps/files-patches` |
| Mail patches | GNOME/Geary(?) | TBD | Lisa hooks | patch-set | `apps/mail-patches` |
| Photos patches | GNOME Photos | TBD | Lisa hooks | patch-set | `apps/photos-patches` |
| Terminal integration | GNOME Console/VTE | TBD | `lisa` CLI presence | integration | `apps/terminal-integration` |

Forks stay **thin, maintained patches in-tree** (build upstream at a
pinned version, drop in our files, apply guarded anchored edits — see
`gnome-control-center-lisa`). We re-pin on a GNOME bump; a moved anchor
fails the build loudly. A fork only earns its own repo if the patch grows
past "thin."

## SDK / libraries (pointers)

`libs/`: `liblisa` (+ gtk/qt), `lisa_ui` (Flutter design system),
`lisa_flutter` (Dart SDK), `forge-harness` (the LisaCode loop),
`lisa-ledger`, `mcp-bus`. Details in the PLAN §5 component map.

## How elementary OS does it (why we differ, for now)

elementary is the closest reference — restrained, humane, its own
identity. Their model:

- **Per-app repos** under one org (`elementary/files`, `/terminal`,
  `/mail`, `/music`, `/tasks`, `/calculator`, `/code`, …). Dozens of small
  repos.
- **A widget/SDK library** — `granite` (their GTK toolkit) — the role
  `lisa_ui` plays for us.
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
`lisa_ui` ≈ Granite; shell surfaces ≈ their shell repos; a thin g-c-c fork
instead of a plug (until a plug-able shell exists). At **M6 / public
alpha**, when Flutter apps are user-installable (the LisaCode/Forge vision:
"everyone can have their own apps") and there's a community, we extract
mature apps to their own repos + a Lisa app catalog — elementary-style.

## Adding something

- **New first-party app:** create `apps/<name>` (Flutter+lisa_ui or GJS),
  add a row above, package it (extend `lisa-shell` for GJS, or a Flutter
  package lane for M6). No new repo until an extraction trigger fires.
- **New fork/patch:** write an ADR (why upstream can't do it unpatched),
  pin the upstream version, keep the patch thin + guarded, add a row to
  *Forked / patched upstream*. Never a silent divergence.
