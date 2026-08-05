# Lisa OS — web presence plan

> **⚠ SUPERSEDED by ADR-0054 (2026-08-05).** This plan was written
> when both domains were parked with no live site and proposed a
> single static page growing into Astro. Reality: two Nuxt 4 sites
> are live (`web/app-nuxt`, `web/dev-nuxt`), the stack is Nuxt UI +
> Tailwind, every colour derives from `branding/tokens.json`, and
> `.github/workflows/web.yml` gates them. **See ADR-0054 and
> `web/README.md`.** Kept for the audience split, which still holds:
> lisaos.app is for people who will run it, lisaos.dev for people
> building on it.


Two domains, two jobs. Companion to `branding/` (identity) and
`docs/VISION.md` (the story). **Updated: 2026-07-23.**

Status: both domains secured 2026-07-23, parked (DNS → 207.207.210.x, no
live site yet). Nothing wired into os-release until `lisaos.dev` is live.

## The split

| Domain | Audience | Job |
|---|---|---|
| **lisaos.app** | everyone | **Marketing** — the story, the feeling, the download. "A computer that's genuinely yours to think with." |
| **lisaos.dev** | users installing / developers building | **The OS itself** — docs, install guide, downloads, SDK, the os-release home link. |

Cross-links: `.app` "Get it" → `.dev/download`; `.dev` header links back
to `.app` for the pitch.

## lisaos.app — marketing

- **Content:** one strong hero (the mark + "A computer that's genuinely
  yours to think with"), the three pillars (local by mechanism / private
  by default / an assistant that's *yours*, the "Her, but yours" angle
  from VISION.md), a few real screenshots (desktop, the Intelligence
  panel, the Ledger), an honest "alpha — here's what works today", and a
  single CTA: **Get Lisa OS** → `lisaos.dev/download` (+ GitHub, + an
  optional email/waitlist).
- **Tone:** warm, restrained, elementary-inspired (design-direction note).
  Uses the brand kit (`branding/`) — Lisa Violet, the wordmark (+ L monogram).
- **Tech:** a single static page to start (self-contained HTML, no
  framework — I can produce this now as an Artifact from the branding).
  Grows into Astro if it needs more pages. Host: GitHub Pages or
  Netlify/Vercel with a `CNAME` for `lisaos.app`.

## lisaos.dev — the OS

- **Content:** docs home (what Lisa is, architecture from PLAN.md),
  **Install** (USB flash + `lisa install`, the iMac path), **Download**
  (points at GitHub Releases — the real artifact host; the update channel
  stays GitHub for now), **SDK/quickstart** (the recipe-extractor sample,
  the OpenAI-compat zero-dep path), **Roadmap/Status** (render ROADMAP.md /
  STATUS.md), and later an **eval dashboard** (M8).
- **Tech:** a docs-site generator over the existing `docs/` tree —
  mdBook or Astro Starlight (Markdown-first, low ceremony). Built in CI,
  published to GitHub Pages with `CNAME lisaos.dev`. This *is* the M8
  "docs site" roadmap item.
- **Identity hook:** once live, point os-release
  `HOME_URL=https://lisaos.dev` and `DOCUMENTATION_URL=https://lisaos.dev`
  (one-line change in `mkosi.postinst.chroot`; hold until the site
  answers so Settings › About never links to a parked page).

## Repos (decided 2026-07-23)

Split, but not symmetrically:

- **`lisaos.app` → its own repo** (`Lisa-AgenticOS/lisaos.app`). Marketing
  moves on its own cadence and should be editable without the OS monorepo
  (elementary keeps `website` separate too). Deploy: Pages/Netlify + CNAME.
- **`lisaos.dev` → built from the monorepo `docs/`**, published out. Docs
  belong next to the code they describe (a behavior change updates docs in
  the same PR); a separate source repo just drifts. Deploy: CI builds
  `docs/` (mdBook / Astro Starlight) → Pages/Cloudflare + CNAME.

Hosting note: GitHub Pages serves one custom domain per repo, so two
domains = two deploys. Cloudflare Pages / Netlify can multi-site from one
account (and point at a monorepo subfolder), which suits the docs deploy.

## Rollout

1. **v1 landing (`lisaos.app`)** — one static page from the branding, a
   real "what works today" + Get CTA. Fastest win; something to point at.
2. **Docs site (`lisaos.dev`)** — Starlight/mdBook over `docs/`, CI →
   Pages, `CNAME`. Then flip os-release URLs to `lisaos.dev`.
3. **Polish** — screenshots (grab from the iMac once the Intelligence
   panel ships), waitlist, eval dashboard.

## Not deciding yet

- Whether the sysupdate channel ever moves off GitHub Releases to a
  `lisaos.dev` endpoint (GitHub is fine and free; revisit at scale).
- Email (`@lisaos.dev`?) — separate from the sites.
