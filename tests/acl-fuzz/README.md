# tests/acl-fuzz — context ACL fuzzing

Spec: `docs/PLAN.md` §5.3, §5.10. Milestone: **M3 gate**.

Adversarial queries against scoped retrieval. Merge-blocking property:
**0 cross-scope leaks** — an app granted `documents.read` never receives
a `mail` chunk, whatever it asks for or how the scope is spelled.

Status: **implemented** (2026-07-26), **rebuilt after mutation testing**
(2026-07-30, issues #115/#116). 81,164 cases, run by `just test` because
the crate is a workspace member.

## What it actually checks

| test | property |
|---|---|
| `the_suite_meets_the_acceptance_size` | §5.3 names 10k queries, so the count is itself gated — a suite that quietly shrank would otherwise still print "ok" |
| `zero_cross_scope_leaks` | every hit's provenance is one the granted scopes permit, across all cases |
| `provenance_unique_terms_never_cross_over` | a term present in exactly one provenance never returns under any other scope |
| `unknown_and_empty_scopes_grant_nothing` | deny by default: `[]`, `""`, `*`, `all`, `admin`, wrong case, padded |
| `junk_alongside_a_valid_scope_changes_nothing` | junk neither widens a valid scope nor voids it |
| `the_corpus_actually_exercises_multi_grants_and_aliases` | the corpus *shape* — size is not coverage |
| `reachable.rs` | the same corpus through `dev.lisaos.Context1.Search`, with the option dictionary fuzzed |

## What mutation testing found (2026-07-30)

The suite was mutation-tested by breaking `search_scoped` eight ways.
Six were caught. Two were not, and both were real leaks:

- **M4** — skip the filter whenever more than one provenance is allowed.
  Survived because **no case in 15,656 ever granted two provenances.**
  The generator tried: `vec!["mail", "file"]` — but `file` is not a scope
  spelling (`files` is), so it silently degraded to `["mail"]` and the
  whole suite ran one-provenance-at-a-time.
- **M5** — make the alias `files.read` also grant `mail`. Survived for
  two reasons, and the second is the interesting one:
  1. no case used any alias spelling; and
  2. **the gate's oracle was the function under test.** It asked
     `allowed_provenance` what was permitted and checked hits against
     that answer, so a mutation moved the expectation and the behaviour
     together.

An oracle derived from the implementation is not an oracle. The expected
mapping now lives in this crate as a literal table (`SCOPE_ALIASES` →
`expected_provenance`), changed by a person who means to change it. With
that, M4 and M5 both fail — as do M3 and M7, which previously only
half-failed.

## Non-vacuity, and why the old floor was not one

"Zero leaks" is satisfied by returning nothing, so the gate asserts a
floor. The old floor was `hits_seen >= 500` against ~4,466 actual — it
would only trip if retrieval fell below **11%** of its volume. Three
floors now:

- total hits, at a fraction of real volume rather than a tenth of it;
- **per provenance** — `web` alone contributed 492 hits, the entire old
  global floor, so a collapse confined to one provenance was invisible;
- **executed cases** — 399 silently vanished into
  `let Ok(…) else { continue }`. If a change made 90% of queries error,
  the suite would shrink to a tenth and still print ok.

## The path a caller can actually reach

`gate.rs` drives `search_scoped`, the ACL-enforcing function. That was
the only thing proven, and the deployed path is
`dev.lisaos.Context1.Search`, which used to select an *unfiltered*
search whenever the caller omitted `scopes` (#100) — which the shipping
overlay did. So the gate could be green while the running system handed
mail chunks to an app with no grant.

`reachable.rs` runs the same corpus over a real p2p connection to the
real interface, fuzzing the **option dictionary** as well as the scopes,
because the dictionary is the attack surface. A p2p peer cannot be
identified, which is exactly the caller to be strictest with.

## Two design rules, both learned the hard way

**The corpus overlaps on purpose.** Every document is about the same
subject — budget, revenue, forecast, margin, a Thursday board review. If
each provenance held distinct vocabulary, a scoped query would return its
own chunks for boring reasons and prove nothing about the filter.

**The suite proves it is not vacuous.** "Zero leaks" is trivially
satisfied by returning nothing, and a green corpus that tests nothing is
exactly what ADR-0029's review rounds found in the guard corpus. So the
gate also asserts a floor on hits actually retrieved and that every
provenance was reachable at least once.

That second rule earned its keep immediately: the first run failed on
`1400`, a term listed as unique to `calendar`. The calendar entry reads
`14:00`, which FTS5 tokenizes as `14` and `00` — so `1400` matched
nothing anywhere, and the "never crosses over" assertion for it would
have passed **vacuously**. Every negative in a suite like this needs a
positive control, or it is decoration.

## Adding cases

Extend `COLLIDING_TERMS`, `PROVENANCE_UNIQUE_TERMS` or `HOSTILE_QUERIES`
in `src/lib.rs`. A new entry in `PROVENANCE_UNIQUE_TERMS` must be
genuinely present in exactly one corpus document — the gate checks that
for you and fails if it is not findable under its own scope.
