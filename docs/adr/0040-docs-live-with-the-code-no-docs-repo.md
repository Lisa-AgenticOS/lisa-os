# ADR-0040: Docs live with the code — there is no docs repo

- Status: **accepted** (decision 2026-08-03, owner asked "should we
  make lisa docs repo", this is the answer)
- Date: 2026-08-03
- Relates: CLAUDE.md rule 10 (document everything we build, only what
  exists), ADR-0039 (the split), #175 (the OS knowledge pack),
  lisaos.dev (the developer portal, task of 2026-07-24)

## Context

The split (ADR-0039) made "where do docs live" a real question: four
repos now exist, and the obvious-looking move is a fifth — `lisa-docs`
— the way many projects do it.

This repo's own history is the argument against. Its most repeated
defect, named in CLAUDE.md rule 10, is documentation describing
something the code does not do — and every instance happened at a
distance: a README for a suite that did not exist, a PLAN line
claiming a nightly built an image no workflow built, a comment
asserting a job ran things it never ran. Distance between the doc and
the thing it describes is the disease vector. A docs repo is that
distance, institutionalized: every code change now needs a second PR
in a second repo, which means it usually will not get one, which means
the docs repo is wrong within a month and *authoritative-looking*
while wrong.

## Decision

**No docs repo. Three layers, each living where its subject lives:**

1. **Component truth** — every component's `README.md`, in the repo
   that holds the component (rule 10's four questions: what it does,
   how it works, how to extend it, its limits). The split repos
   already carry this; their READMEs were corrected the day a review
   caught them overclaiming (#173), which is the maintenance loop
   working — possible only because the doc sits next to the code in
   the same commit.
2. **Decisions and state** — PLAN, ADRs, STATUS stay in `lisa-os`,
   which remains the architectural center (the never-split list,
   ADR-0006). Split repos reference `lisa-os` ADRs by number rather
   than copying them; one record, no forks of the record.
3. **Rendered, user-facing docs** — **lisaos.dev is the docs site.**
   It already exists as the developer portal; what it renders is
   *generated* from the source repos (the same generation #175 builds
   for the on-device knowledge pack — one curation step, two
   consumers: the model on the device and the website). Nothing on
   lisaos.dev is hand-written twice.

The knowledge-pack generator (#175) is therefore also the docs-site
generator. That is the piece that makes this ADR cheap instead of
pious: rule-10 READMEs are the single source, and both the site and
the model consume them mechanically.

## Consequences

- A doc change is reviewable in the same diff as the code it
  describes, and CI that touches one can gate on the other.
- lisaos.dev gains a build step that pulls from four repos. That is a
  generator run, not an editorial process — if it needs editing before
  publish, the fix belongs in the source README, never in the output.
- The split repos stay documentation-light by design: component
  READMEs plus a pointer home. Anyone landing in `lisa-apps` is one
  link from PLAN.
- If a real handbook need appears later (long-form guides that belong
  to no component), the answer is a `docs/guides/` directory in
  `lisa-os` feeding the same generator — still not a repo.

## What would change this

External contributors writing substantial narrative docs at a cadence
that swamps `lisa-os` review, or a translation effort — both are
coordination problems a dedicated repo genuinely helps. Neither
exists today.
