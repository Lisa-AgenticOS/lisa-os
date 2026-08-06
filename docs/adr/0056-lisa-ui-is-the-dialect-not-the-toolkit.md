# ADR-0056 — `lisa_ui` is the dialect, not the toolkit

- **Status:** accepted, partially executed — **step 1 landed
  2026-08-06**: the Agent Bus edge is one file at
  `apps/lisa_ui/mcp/protocol.js` and Mail, Surfer and Preview import it.
  Steps 2–4 (token sheet loading, `LisaWindow`, widgets) are unbuilt, so
  #282 is not closed.
- **Date:** 2026-08-06
- **Supersedes in part:** ADR-0014 (the Dart `lisa_ui`), which ADR-0047
  had already parked.
- **Amended 2026-08-06:** the library lives at `apps/lisa_ui`, not
  `libs/lisa_ui` — see "Where it actually lives" below.
- **Claims:**
  - `path:apps/lisa_ui/mcp/protocol.js` — step 1: one copy of the Agent Bus edge
  - `absent:libs/lisa_ui` — the `libs/` path this ADR originally named is still unused, deliberately; see the amendment

## Context

Eight GJS surfaces ship today — Mail, Surfer, Preview, Notes, the
Assistant, the Ledger app, Settings, the Terminal integration — and
they do not look like one system. #282 is the user report: *"close
button is on the app title bar same place always, like if you compare
main and Surfer it's strangely different, dark/light acts strange."*

Two things were measured on 2026-08-06 rather than assumed:

1. **`mcp-protocol.js` and `mcp.js` exist in three copies each**, across
   Mail, Surfer and Preview, with three different hashes and three
   different lengths (73/65/57 and 75/87/102 lines). They have not
   merely been copied — they have **drifted**. PLAN §5.12 already
   recorded the consequence: this triplication is why **#218 had to be
   found and fixed three times**, and #219 more than once.
2. That edge is a **security boundary**, not a convenience. It is where
   a tool result's provenance is stamped. #302 (dda3c9a) and #313 are
   both, at root, defects of a boundary that exists in triplicate.

`branding/tokens.json` already exists, already generates a GJS module
and a CSS sheet, and is already gated by `check-tokens.py` over
`shell/`, `apps/` and `web/`. The design system is not the missing
piece. The missing piece is a place for shared *code*.

## Decision

Build `libs/lisa_ui` as the shared GJS/GTK4 library for Lisa surfaces —
**a dialect above libadwaita, never a replacement for it.** The
analogy is Material or Cupertino to Flutter: a widget set and a theme
layered on a toolkit that stays foundation. Rule 11 is unchanged —
GTK4/libadwaita and Mutter are never forked.

### It is read at runtime, not compiled in

This is the one place the analogy deliberately breaks, and in our
favour. Material is compiled into every Flutter app; updating it means
rebuilding them. GJS is interpreted, so the library is installed at a
well-known path and resolved when an app starts:

    /usr/share/lisa/ui/

**Explicitly not GResource for the app-facing surface.** GResource
compiles assets into a binary, which would reintroduce exactly the
rebuild coupling this decision exists to avoid.

What that buys, stated precisely so nobody over-claims it later:

| Layer | Update propagation |
|---|---|
| Tokens + theme (CSS) | **Live** — a running app can restyle on a `Gio.FileMonitor` |
| Widgets (ES modules) | **Next launch** — GJS caches ES modules; there is no hot reload |

So "ship glass and every app is glass" is true for the theme layer and
false for the widget layer. Both halves go in the README.

### Where it actually lives (amended 2026-08-06)

`apps/lisa_ui`, not `libs/lisa_ui`, and the reason is the shipped tree
rather than taste. `os/repo-tools/build-apps-payload.sh` FLATTENS
`apps/<app>` and `shell/<surface>` into one payload root, so
`apps/mail/lib/x.js` is three levels below the repo root and
`mail/lib/x.js` is two below the payload root. A relative import can
only mean the same thing in both trees if the library sits beside the
consumer in each — hence `../../lisa_ui/...`, resolving in the repo and
on a device without a build step, a generated copy, or a path rewritten
at package time.

Every consumer of step 1 is an app, so this costs nothing today. It has
a known edge: **a `shell/` surface cannot import it by that path**, and
the honest answer when one needs to is to fix the depth — by staging
into `apps/` and `shell/` subdirectories, or by resolving at runtime —
NOT by adding a second copy. `apps/lisa_ui/README.md` carries that
warning where somebody about to hit it will read it.

### Version skew is the cost, and it is structural

The library ships with the OS image (A/B slots) while apps ship on the
app channel (ADR-0020). **Different cadences by design**, which means
an app will eventually run against a library it was not written for.
Flutter avoids this by compiling in; we are choosing the other trade
and therefore owe it a mechanism:

- the library declares an API version;
- each app manifest declares the minimum it needs;
- a mismatch is a **loud refusal at launch**, never a silent
  misrender.

Fail closed and say why — the same shape as the guard work, for the
same reason.

### One package, built from the monorepo

`lisa_ui` is consumed by the shell (lisa-desktop), the apps
(lisa-apps) and this repo. That is the same ADR-0039 seam that produced
lisa-desktop#7, where two packages own the same files with no
`conflicts=`. Duplicating the library the way the surfaces were
duplicated is the failure mode to avoid, so: **one package, built
here**, and #7 is resolved as part of this work rather than beside it.

### The name

The Dart `libs/lisa_ui` and `libs/lisa_flutter` were deleted on
2026-08-06 (d1bdc18). They were the first idea — build the apps in
Flutter — and ADR-0047 chose GJS. Keeping two kits one underscore apart
was a trap: CLAUDE.md carried a standing warning not to import the
wrong one, which is a documentation fix for a naming problem. ADR-0014
and ADR-0047 keep their text; a lane somebody chose not to take is
still a decision.

## Build order, by evidence

1. **The Agent Bus edge** — `mcp-protocol.js` + `mcp.js`, absorbing the
   three drifted copies. A security boundary that has already cost
   three fixes of one bug, and that #313 shows is currently held up by
   three copies of a workaround.
2. **The token sheet** — already generated, already gated. Needs one
   loading path.
3. **`LisaWindow`** — closes #282. Header-bar structure and window
   controls become impossible to get wrong, rather than a convention
   eight apps each re-implement. Dark/light stops being per-app.
4. **Widgets — only once a second app needs one.** A widget set
   invented up front is a guess; one extracted from two real callers is
   a fact. Migration is per-module, when someone is already touching the
   file.

## Sequencing, and why this is not started yet

The harness reality check (2026-08-06) found that the injection gate
never imported `bus_tools` — it proved the bus escalates a chain and
never that the loop produces one — and that only `web` provenance
tainted a run. Those are #302 (fixed), #303/#304 (in progress) and
#288 (fixed for harnessd).

Step 1 of this ADR **moves the code those fixes touch**. Starting the
library while the boundary underneath it is being repaired would mean
migrating a moving target and reviewing both changes at once. So the
library waits until the harness work is landed and green. That is a
sequencing decision, not a hedge.

## What this ADR does not decide

- Whether `lisa_ui` also becomes the home for non-UI app plumbing. The
  mcp edge is UI-adjacent at best, and the package may deserve
  namespaces (`ui/`, `mcp/`) or a more honest name than "ui". Decide
  when step 1 lands and the shape is visible.
- The widget inventory. Deliberately — see step 4.
- Whether the shell surfaces in `lisa-desktop` consume the same package
  or a subset. That falls out of resolving lisa-desktop#7.

## Consequences

- #282 becomes closable by construction rather than by eight
  convergent edits.
- #218's class of bug — fix it here, forget it there — stops being
  possible at the Agent Bus edge.
- A design change ships as a file, not as eight app releases.
- We take on version skew, and it must be enforced rather than hoped
  for. If the API-version check is not built in step 1, this ADR has
  traded a known problem for a quieter one.
