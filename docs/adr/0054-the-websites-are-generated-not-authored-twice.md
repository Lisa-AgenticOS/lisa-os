# ADR-0054 — The websites are generated from the repo, not authored twice

- **Status:** accepted, not implemented — the direction is decided; the
  phases below are tracked as one issue and none has landed.
- **Date:** 2026-08-05

## Context

Two sites exist: `lisaos.app` (marketing) and `lisaos.dev` (developer
portal). Both are Nuxt 4. They are about to grow — docs per component,
an API reference, release notes, downloads, design, contribution —
and the shape they have today does not survive that growth. Three
things were measured on 2026-08-05 rather than assumed:

1. **Docs are hand-written `.vue` files.** `app/pages/docs/*.vue`
   carry prose as Vue templates. Meanwhile `docs/` in the monorepo is
   the actual source of truth (CLAUDE.md rules 1 and 10) — so the same
   sentences exist twice, in two languages, with nothing keeping them
   equal. At five pages it is annoying; at fifty it is a second
   documentation set that silently contradicts the first.
2. **Hand-written content goes stale immediately.** The portal's
   "What shipped this week" ended at Aug 3 while the repo had shipped
   a release, the ports lane, a package rename and a four-repo CI
   review. Nobody lied; a human list simply stopped being written.
3. **The sites have NO CI.** No workflow builds them, checks their
   links, or deploys them. Every deploy is a person running
   `bp deploy web/…` by hand — the same undocumented-manual-step shape
   that made the `[lisa]` publish chain fragile (#171).

There is also a design-system duplication: `main.css` maintains a
Tailwind `@theme` colour ramp *and* a parallel `:root` token set, and
neither derives from `branding/tokens.json` — the repo's single source
for surface colour, whose lint gate covers `shell` and `apps` but not
`web`. The sites can drift off-brand and no gate would notice.

## Decision

**Anything that exists in the repo is rendered by the site, never
retyped into it. Anything that can be derived is derived. What remains
hand-written is argument and design — the two things a generator
cannot produce.**

### 1. Content pipeline — docs are markdown, in the repo

`@nuxt/content` renders markdown; the docs themselves live in `docs/`
in the monorepo where they already are, and where component READMEs
(rule 10) already sit beside the code they describe. A `.vue` page per
doc is retired. Contributors write markdown, not Vue. The ADR index
and knowledge pack are already generated from those files, so the site
becomes the third consumer of one source rather than a second copy.

### 2. Derived, not typed

| Surface | Source of truth |
|---|---|
| Release notes / news | GitHub Releases API + `docs/STATUS.md` |
| Downloads + checksums | GitHub Releases API |
| API reference | the D-Bus interfaces, MCP manifests and OpenAPI shape in source |
| Screenshots | the CI screenshot artifacts (already generated) |
| Good-first-issues board | GitHub Issues API (already live) |
| Design tokens | `branding/tokens.json` |

If a page can go stale, it must be derived or dated. A "what shipped"
list that a person maintains is a promise nobody keeps.

### 3. One token system, gated

`branding/tokens.json` → Tailwind `@theme` → utilities. The parallel
`:root` set is deleted. `check-tokens.py` gains `web` in its SURFACES
list, so an unsanctioned hex on the websites is the same red build it
already is in the shell and the apps.

### 4. The stack is Nuxt UI + Tailwind, fully — and the house style is
a THEME on top of it, not a replacement for it

Measured: today the sites use **zero** Nuxt UI components and
essentially zero Tailwind utilities while shipping ~206 KB of both.
The cost is already paid. The first draft of this ADR proposed a
middle path — utilities for layout, Nuxt UI for a few interactive
bits, bespoke CSS for the rest — and that was the wrong instinct: a
half-used framework is the worst of both, because it carries the full
weight while a private dialect still has to be learned, maintained and
debugged by whoever comes next.

So: **all of it.**

- **Page primitives from Nuxt UI** — header, footer, hero, section,
  card, grid, the content navigation and the docs search. These are
  the exact shapes a marketing site and a documentation portal need,
  and they arrive with keyboard behaviour, focus management, ARIA and
  dark mode already correct.
- **Interactive components from Nuxt UI** — dropdowns, dialogs, tabs,
  command palette, toasts. Hand-rolling these is how accessibility
  quietly fails.
- **Composables from Nuxt** — `useColorMode`, `defineShortcuts`,
  `useAsyncData`, and Content's query/navigation composables. State
  and data-fetching are solved problems here; solving them again is
  how the two sites drift apart.
- **Everything else in Tailwind utilities.** Three of the four layout
  bugs found on 2026-08-05 were bespoke-CSS defects that utilities
  make structurally impossible: a grid declaring three columns against
  two-child markup, a `display:none` that lost to a media query on
  source order, and a negative inset that overflowed the viewport. The
  cascade is the bug surface; utilities remove it.

**The house style survives as configuration, not as a fork.** The
drawing-sheet identity — the twelve-column zones, the graph ground,
the numbered fields, the title block — is expressed as Tailwind
`@theme` tokens plus an `app.config.ts` Nuxt UI theme plus a handful
of composed utility classes. A component is only written by hand when
Nuxt UI has no equivalent, and it is built out of the same tokens.

**Raw CSS is reserved for the four things utilities genuinely cannot
say**: the column-guide gradients, `counter()` zone numbering,
`animation-timeline: view()`, and `grid-auto-rows: calc(100cqw/12)`.
Roughly forty lines, in `@layer`, each with a comment saying why it is
not a utility.

**Why this is the futureproof choice, stated plainly:** accessibility
fixes, new components and browser-behaviour changes arrive as a
version bump instead of a bug report nobody files; a contributor who
knows Nuxt knows this codebase on day one; and the design system has
one vocabulary — tokens — instead of a token file, a `:root` set, and
250 lines of bespoke selectors that only their author can safely
change.

### 5. The sites get CI

A `web.yml` workflow: build both sites on any PR touching `web/**`,
run a link check over the built output, and deploy on merge to `main`
via the scoped basepod deploy token that already exists. Manual
`bp deploy` becomes the break-glass path, not the normal one.

## Phases

0. **Foundation.** `branding/tokens.json` → Tailwind `@theme` →
   `app.config.ts` Nuxt UI theme; the parallel `:root` set deleted;
   `check-tokens.py` covers `web`; `web.yml` builds both sites on PR,
   link-checks the output and deploys on merge. Correctness first —
   this is the only phase that closes a live gap.
1. **Adopt the framework.** Pages rebuilt on Nuxt UI page primitives
   with the drawing-sheet identity carried by tokens and theme. The
   bespoke CSS shrinks to the four documented exceptions.
2. **Content pipeline.** `@nuxt/content`; `docs/*.md` from the repo
   rendered by the portal; the `.vue` doc pages retired.
3. **Derived surfaces.** News, downloads and the API reference
   generated from Releases, the API and source.
4. **Scale.** Docs search via the Nuxt UI command palette; versioned
   docs once there is more than one release line worth documenting.

Phases 0 and 1 are the ones that make the rest cheap; nothing later
should begin while the token duplication or the missing CI is still
in place, because every page added under the old shape is a page that
has to be converted twice.

## Consequences

- Contributors write documentation in markdown next to the code, and
  the website is a rendering of the repo rather than a parallel
  publication. Rule 10's "document only what exists" gets a mechanism
  instead of a habit.
- A doc that moves breaks a link at PR time rather than in public.
- The websites stop being the one part of this project that ships
  without a gate — which, on a project whose entire argument is that
  the system shows its work, was the wrong exception to have.
- Nuxt UI stays installed and mostly unused until phase 3 spends it on
  search. That is deliberate: it is already paid for, and rewriting
  the component layer twice would cost more than waiting.
