//! The merge-blocking context ACL gate (`docs/PLAN.md` §5.3 acceptance).
//!
//! Property: **zero cross-scope leaks.** Every hit returned for a set of
//! granted scopes carries a provenance those scopes permit — regardless
//! of ranking, query syntax, or how the scope was spelled.
//!
//! The suite also proves it is not vacuous. "Zero leaks" is trivially
//! satisfied by returning nothing, and a corpus that is green because it
//! tests nothing is exactly the failure ADR-0029's review rounds found in
//! the guard corpus. So the gate asserts a floor on hits actually
//! retrieved, and that each provenance was reachable at least once.

use lisa_acl_fuzz::{
    PROVENANCE_UNIQUE_TERMS, PROVENANCES, SCOPE_ALIASES, cases, expected_provenance, seeded_store,
};
use std::collections::BTreeSet;

fn store() -> (tempfile::TempDir, lisa_contextd::ContextStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_store(&dir.path().join("ctx.db"));
    (dir, store)
}

/// PLAN §5.3 names a number — 10k adversarial queries — so the number is
/// part of the acceptance, not a footnote. A suite that quietly shrank to
/// a few hundred cases would still print "ok".
#[test]
fn the_suite_meets_the_acceptance_size() {
    let n = cases().len();
    assert!(
        n >= 10_000,
        "§5.3 acceptance is 10k adversarial queries; this suite runs {n}"
    );
}

/// Size is not coverage (issue #115).
///
/// The suite ran 15,656 cases and every one of them permitted **at most
/// one provenance**, because the single multi-scope case in the
/// generator used `"file"` — which is not a scope spelling, `"files"`
/// is — and silently degraded to one. It also never used any of the
/// alias spellings the ACL accepts. Two mutations survived on that:
/// skipping the filter whenever more than one provenance is allowed
/// (M4), and making `files.read` also grant `mail` (M5).
///
/// So the shape of the corpus is asserted, not assumed. This is the
/// test that would have failed on the old generator.
#[test]
fn the_corpus_actually_exercises_multi_grants_and_aliases() {
    let cases = cases();

    let mut widest = 0usize;
    let mut multi_cases = 0usize;
    let mut aliases_used: BTreeSet<&str> = BTreeSet::new();
    let canonical: BTreeSet<&str> = PROVENANCES.iter().map(|(_, s)| *s).collect();

    for (_, scopes) in &cases {
        let refs: Vec<&str> = scopes.iter().map(String::as_str).collect();
        let allowed = expected_provenance(&refs);
        widest = widest.max(allowed.len());
        if allowed.len() > 1 {
            multi_cases += 1;
        }
        for s in &refs {
            if !canonical.contains(s) && SCOPE_ALIASES.iter().any(|(_, a)| a == s) {
                aliases_used.insert(SCOPE_ALIASES.iter().find(|(_, a)| a == s).unwrap().1);
            }
        }
    }

    assert!(
        widest >= PROVENANCES.len(),
        "the widest grant in the corpus permits {widest} provenance(s); a filter          that gives up when several are allowed would never be caught"
    );
    assert!(
        multi_cases >= 1_000,
        "only {multi_cases} cases permit more than one provenance —          multi-grant is the shape M4 hid in"
    );

    let all_aliases: BTreeSet<&str> = SCOPE_ALIASES
        .iter()
        .map(|(_, a)| *a)
        .filter(|a| !canonical.contains(a))
        .collect();
    assert_eq!(
        aliases_used, all_aliases,
        "some alias spelling is never exercised; an untested match arm is an          untested rule (M5)"
    );
}

#[test]
fn zero_cross_scope_leaks() {
    let (_dir, store) = store();
    let cases = cases();

    let mut leaks: Vec<String> = Vec::new();
    let mut hits_seen = 0usize;
    let mut executed = 0usize;
    let mut reached: BTreeSet<String> = BTreeSet::new();
    let mut per_provenance: std::collections::BTreeMap<String, usize> = Default::default();

    for (query, scopes) in &cases {
        let refs: Vec<&str> = scopes.iter().map(String::as_str).collect();
        // A malformed FTS query is allowed to error; it is NOT allowed to
        // return a chunk the scopes do not permit.
        let Ok(hits) = store.search_scoped(query, &refs, 20) else {
            continue;
        };
        executed += 1;
        // The oracle is this crate's own table, NOT the ACL's function
        // (#115). Asking the code under test what it should have done
        // makes every mutation of that answer invisible: M5 moved the
        // expectation and the behaviour together and passed green.
        let allowed = expected_provenance(&refs);
        for h in hits {
            hits_seen += 1;
            reached.insert(h.provenance.clone());
            *per_provenance.entry(h.provenance.clone()).or_default() += 1;
            if !allowed.contains(&h.provenance) {
                leaks.push(format!(
                    "  scopes {scopes:?} (allow {allowed:?}) query {query:?} -> {} from {}",
                    h.provenance, h.source
                ));
            }
        }
    }

    assert!(
        leaks.is_empty(),
        "{} cross-scope leak(s) across {} cases:\n{}",
        leaks.len(),
        cases.len(),
        leaks.join("\n")
    );

    // Non-vacuity, three ways (issue #116).
    //
    // The floor used to be `hits_seen >= 500` against ~4,466 actual —
    // it would only trip if retrieval fell below 11% of its volume. A
    // change that silently dropped 89% of hits still "proved
    // non-vacuous".
    assert!(
        hits_seen >= 4_000,
        "only {hits_seen} hits across {} cases — the suite is not exercising retrieval, \
         so 'zero leaks' proves nothing",
        cases.len()
    );

    // A case that ERRORED is a case that did not run. 399 of them
    // vanished into `let Ok(…) else { continue }`, and nothing counted
    // them: if a change made 90% of queries error, the suite would
    // shrink to a tenth and still print ok.
    assert!(
        executed * 10 >= cases.len() * 9,
        "only {executed} of {} cases actually ran ({} errored) — a suite that \
         quietly stops executing still reports zero leaks",
        cases.len(),
        cases.len() - executed
    );

    // Per provenance, not just in total: `web` alone contributed 492
    // hits, which was the entire old global floor. A collapse confined
    // to one provenance was invisible.
    for (provenance, _) in PROVENANCES {
        let n = per_provenance.get(*provenance).copied().unwrap_or(0);
        assert!(
            n >= 200,
            "only {n} hits from `{provenance}` — a collapse confined to one \
             provenance hides inside a global floor"
        );
    }

    let expected: BTreeSet<String> = PROVENANCES.iter().map(|(p, _)| p.to_string()).collect();
    assert_eq!(
        reached, expected,
        "some provenance was never retrieved at all; the corpus or the scope map has drifted"
    );
}

/// The sharpest signal available: a term that exists in exactly one
/// provenance must never come back under any *other* scope. A hit here
/// could only have come from the wrong place.
#[test]
fn provenance_unique_terms_never_cross_over() {
    let (_dir, store) = store();
    let mut leaks = Vec::new();
    let mut proved = 0usize;

    for (term, owner) in PROVENANCE_UNIQUE_TERMS {
        // It must be findable under its OWN scope, or the term is not
        // actually in the corpus and the negative below is meaningless.
        let own_scope = PROVENANCES
            .iter()
            .find(|(p, _)| p == owner)
            .map(|(_, s)| *s)
            .expect("owner provenance has a scope");
        let own = store.search_scoped(term, &[own_scope], 20).unwrap();
        assert!(
            own.iter().any(|h| &h.provenance == owner),
            "`{term}` is supposed to be unique to {owner} but is not findable there"
        );
        proved += 1;

        for (other, scope) in PROVENANCES {
            if other == owner {
                continue;
            }
            for h in store.search_scoped(term, &[scope], 20).unwrap() {
                leaks.push(format!(
                    "  `{term}` (unique to {owner}) returned {} from {} under scope {scope}",
                    h.provenance, h.source
                ));
            }
        }
    }

    assert!(leaks.is_empty(), "cross-over leaks:\n{}", leaks.join("\n"));
    assert_eq!(proved, PROVENANCE_UNIQUE_TERMS.len());
}

/// Deny by default: no scopes, unknown scopes, and scope names that look
/// like wildcards all grant nothing.
#[test]
fn unknown_and_empty_scopes_grant_nothing() {
    let (_dir, store) = store();
    for scopes in [
        vec![],
        vec![""],
        vec!["*"],
        vec!["all"],
        vec!["admin"],
        vec!["inference"],
        vec!["documents.write"],
        vec!["DOCUMENTS.READ"], // case matters; the map is exact
        vec![" documents.read "],
        vec!["totally.bogus", "also.bogus"],
    ] {
        let hits = store
            .search_scoped("budget revenue forecast", &scopes, 20)
            .unwrap();
        assert!(
            hits.is_empty(),
            "scopes {scopes:?} granted {} hit(s): {hits:?}",
            hits.len()
        );
    }
}

/// A known scope mixed with junk grants exactly what the known scope
/// grants — the junk neither widens nor voids it.
#[test]
fn junk_alongside_a_valid_scope_changes_nothing() {
    let (_dir, store) = store();
    for (provenance, scope) in PROVENANCES {
        let clean = store.search_scoped("budget revenue", &[scope], 20).unwrap();
        let mixed = store
            .search_scoped("budget revenue", &[scope, "totally.bogus", ""], 20)
            .unwrap();
        assert_eq!(
            clean.len(),
            mixed.len(),
            "junk scopes changed the result count for {scope}"
        );
        for h in mixed {
            assert_eq!(&h.provenance, provenance, "junk scope widened {scope}");
        }
    }
}
