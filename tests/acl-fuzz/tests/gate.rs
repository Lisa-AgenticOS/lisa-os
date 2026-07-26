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

use lisa_acl_fuzz::{PROVENANCE_UNIQUE_TERMS, PROVENANCES, cases, seeded_store};
use lisa_contextd::acl::allowed_provenance;
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

#[test]
fn zero_cross_scope_leaks() {
    let (_dir, store) = store();
    let cases = cases();

    let mut leaks: Vec<String> = Vec::new();
    let mut hits_seen = 0usize;
    let mut reached: BTreeSet<String> = BTreeSet::new();

    for (query, scopes) in &cases {
        let refs: Vec<&str> = scopes.iter().map(String::as_str).collect();
        // A malformed FTS query is allowed to error; it is NOT allowed to
        // return a chunk the scopes do not permit.
        let Ok(hits) = store.search_scoped(query, &refs, 20) else {
            continue;
        };
        let allowed = allowed_provenance(&refs);
        for h in hits {
            hits_seen += 1;
            reached.insert(h.provenance.clone());
            if !allowed.contains(&h.provenance.as_str()) {
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

    // Non-vacuity: the filter must have been exercised, not merely quiet.
    assert!(
        hits_seen >= 500,
        "only {hits_seen} hits across {} cases — the suite is not exercising retrieval, \
         so 'zero leaks' proves nothing",
        cases.len()
    );
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
