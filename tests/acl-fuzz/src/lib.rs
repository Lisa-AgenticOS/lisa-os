//! The context ACL fuzz corpus (`docs/PLAN.md` §5.3, §5.10).
//!
//! The property under test: **an app granted `documents.read` never
//! receives a `mail` chunk** — not as a best hit, not through ranking,
//! not through a crafted query, not through a scope spelled oddly.
//!
//! Two things make a fuzz suite like this worth having rather than
//! decorative, and both are lessons from ADR-0029's review rounds:
//!
//! 1. **The corpus must overlap.** If each provenance held distinct
//!    vocabulary, a scoped query would return its own chunks for boring
//!    reasons and prove nothing. Every document here is written about the
//!    *same* subject, so a weak filter leaks immediately.
//! 2. **The suite must prove it is not vacuous.** "Zero leaks" is
//!    trivially achievable by returning nothing at all. The gate
//!    therefore also asserts a floor on hits actually retrieved — a green
//!    run means the filter was exercised, not merely quiet.

use lisa_contextd::ContextStore;

/// Every provenance the ACL knows, with a scope that grants it.
///
/// `screen` is `screen.read`, not `screen.once` (issue #112):
/// `screen.once` is the portal's *per-invocation* scope and no longer
/// grants a provenance-wide read, so a suite that used it would be
/// asserting against a scope that grants nothing. The gate's own
/// non-vacuity floor caught the change, which is the floor doing its
/// job.
pub const PROVENANCES: &[(&str, &str)] = &[
    ("file", "documents.read"),
    ("mail", "mail.read"),
    ("calendar", "calendar.read"),
    ("screen", "screen.read"),
    ("web", "web.read"),
];

/// Documents deliberately written about ONE subject across every
/// provenance, so any weakness in the filter surfaces as a leak rather
/// than hiding behind vocabulary that never collides.
pub const CORPUS: &[(&str, &str, &str)] = &[
    (
        "/docs/q3-report.md",
        "file",
        "quarterly budget report: revenue forecast, headcount plan and the \
         margin numbers for the board review on Thursday",
    ),
    (
        "/docs/notes.txt",
        "file",
        "budget review notes — revenue looks soft, forecast needs a second pass",
    ),
    (
        "mail://inbox/42",
        "mail",
        "Re: budget — the revenue forecast looks off before the board review, \
         can we talk Thursday? password reset link enclosed",
    ),
    (
        "mail://inbox/77",
        "mail",
        "Fwd: headcount plan and margin numbers, confidential, do not forward",
    ),
    (
        "calendar://event/9",
        "calendar",
        "Board review Thursday 14:00 — budget, revenue forecast, headcount. \
         Attendees confirmed, room Vasquez booked",
    ),
    (
        "screen://capture/1",
        "screen",
        "spreadsheet on screen showing budget revenue forecast and margin numbers",
    ),
    (
        "https://example.com/pricing",
        "web",
        "public pricing page: revenue model, forecast methodology and margin bands",
    ),
];

/// A store loaded with the overlapping corpus.
pub fn seeded_store(path: &std::path::Path) -> ContextStore {
    let store = ContextStore::open(path).expect("open context store");
    for (source, provenance, content) in CORPUS {
        store
            .add_document(source, provenance, content)
            .expect("seed document");
    }
    store
}

/// Terms every document shares — these are the queries most likely to
/// surface a leak, because every provenance matches them.
pub const COLLIDING_TERMS: &[&str] = &[
    "budget",
    "revenue",
    "forecast",
    "margin",
    "headcount",
    "board",
    "review",
    "thursday",
    "numbers",
    "plan",
    "budget revenue",
    "revenue forecast",
    "board review",
    "margin numbers",
    "budget OR revenue",
    "forecast AND margin",
];

/// Terms that exist in exactly one provenance. Querying these under a
/// *different* scope must return nothing — the sharpest leak signal
/// available, because a hit can only have come from the wrong place.
pub const PROVENANCE_UNIQUE_TERMS: &[(&str, &str)] = &[
    ("password", "mail"),
    ("confidential", "mail"),
    ("forward", "mail"),
    ("quarterly", "file"),
    ("soft", "file"),
    ("spreadsheet", "screen"),
    ("pricing", "web"),
    ("methodology", "web"),
    ("bands", "web"),
    // NOT "1400": the calendar entry reads "14:00", which FTS5 tokenizes
    // as `14` and `00`. Querying `1400` finds nothing anywhere, so the
    // "never crosses over" check would have passed vacuously. The gate's
    // findable-under-its-own-scope assertion caught it, which is the
    // whole reason that assertion exists.
    ("attendees", "calendar"),
    ("vasquez", "calendar"),
];

/// Query shapes aimed at the retrieval layer itself rather than at the
/// content: FTS5 operators, column filters, quoting, and SQL-ish
/// injection. None of these may widen the provenance filter, which is a
/// separate SQL predicate — but "should not" is what a fuzz suite exists
/// to check rather than assume.
pub const HOSTILE_QUERIES: &[&str] = &[
    "budget OR mail",
    "budget NOT file",
    "provenance:mail",
    "provenance : mail",
    "d.provenance:mail",
    "\"budget\" OR \"password\"",
    "budget*",
    "*",
    "^budget",
    "NEAR(budget password, 100)",
    "budget NEAR password",
    "{budget password}",
    "budget' OR '1'='1",
    "budget' OR 1=1 --",
    "'; SELECT * FROM documents; --",
    "budget\") OR provenance IN (\"mail\") --",
    "chunks MATCH 'password'",
    "budget AND (password OR confidential)",
    "-file password",
    "budget -file",
    "\"",
    "\"\"",
    "(",
    ")",
    "()",
    "",
    "   ",
    "\0",
    "budget\u{0}password",
    "\u{202e}budget",
    "БЮДЖЕТ",
    "予算",
    "🔥budget",
];

/// Every alias `provenance_for_scope` accepts, by the provenance it
/// grants.
///
/// The suite used only the five canonical spellings, so **no case in
/// 15,656 ever exercised an alias** — and a mutation making
/// `files.read` also grant `mail` passed green (issue #115, M5). An
/// alias is a separate arm of a match statement; an untested arm is an
/// untested rule.
pub const SCOPE_ALIASES: &[(&str, &str)] = &[
    ("file", "documents.read"),
    ("file", "files.read"),
    ("file", "documents"),
    ("file", "files"),
    ("mail", "mail.read"),
    ("mail", "mail"),
    ("calendar", "calendar.read"),
    ("calendar", "calendar"),
    ("screen", "screen.read"),
    ("screen", "screen"),
    ("web", "web.read"),
    ("web", "web"),
];

/// What a set of scopes SHOULD grant, computed from this crate's own
/// table rather than from the code under test.
///
/// This is the fix that mattered (issue #115, M5). The gate used to ask
/// `lisa_contextd::acl::allowed_provenance` what was permitted and then
/// check the hits against that answer — so a mutation making
/// `files.read` also grant `mail` moved the *expectation* alongside the
/// behaviour, and returning mail chunks under a documents grant was
/// green. The suite was checking the implementation against itself.
///
/// An oracle has to be independent or it is not an oracle. This one is
/// a literal table: [`SCOPE_ALIASES`], written down here, changed by a
/// person who means to change it.
pub fn expected_provenance(scopes: &[&str]) -> std::collections::BTreeSet<String> {
    scopes
        .iter()
        .flat_map(|s| {
            SCOPE_ALIASES
                .iter()
                .filter(move |(_, alias)| alias == s)
                .map(|(p, _)| (*p).to_string())
        })
        .collect()
}

/// Scope spellings, including the ones an ACL is most likely to get
/// wrong: unknown, empty, duplicated, case-varied, whitespace-padded,
/// a known scope mixed with junk, every alias, and **combinations**.
pub fn scope_variants() -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    for (_, scope) in PROVENANCES {
        out.push(vec![scope.to_string()]);
        out.push(vec![scope.to_string(), scope.to_string()]);
        out.push(vec![scope.to_uppercase()]);
        out.push(vec![format!(" {scope} ")]);
        out.push(vec![scope.to_string(), "totally.bogus".to_string()]);
        out.push(vec!["totally.bogus".to_string(), scope.to_string()]);
    }
    // Every alias, alone and beside junk. Untested match arms were how
    // M5 hid.
    for (_, alias) in SCOPE_ALIASES {
        out.push(vec![alias.to_string()]);
        out.push(vec![alias.to_string(), "totally.bogus".to_string()]);
    }
    // MULTI-PROVENANCE GRANTS. There was exactly one attempt at this —
    // `vec!["mail", "file"]` — and `file` is not a scope spelling
    // (`files` is), so it silently degraded to `["mail"]` alone and the
    // whole suite ran with at most one provenance allowed. A mutation
    // that skipped filtering whenever more than one provenance was
    // permitted therefore passed green (#115, M4).
    //
    // Every ordered pair, canonical and aliased, plus the full set.
    for (_, a) in PROVENANCES {
        for (_, b) in PROVENANCES {
            if a != b {
                out.push(vec![a.to_string(), b.to_string()]);
            }
        }
    }
    for (pa, alias_a) in SCOPE_ALIASES {
        for (pb, alias_b) in SCOPE_ALIASES {
            if pa != pb {
                out.push(vec![alias_a.to_string(), alias_b.to_string()]);
            }
        }
    }
    // Three, and everything: the widest legitimate grant still filters.
    out.push(
        PROVENANCES
            .iter()
            .map(|(_, s)| s.to_string())
            .collect::<Vec<_>>(),
    );
    out.push(vec![
        "documents.read".into(),
        "mail.read".into(),
        "web.read".into(),
    ]);
    // …and the widest grant with junk in the middle.
    out.push(vec![
        "documents.read".into(),
        "totally.bogus".into(),
        "mail.read".into(),
    ]);
    // Deny-by-default shapes.
    out.push(vec![]);
    out.push(vec!["".to_string()]);
    out.push(vec!["inference".to_string()]);
    out.push(vec!["*".to_string()]);
    out.push(vec!["all".to_string()]);
    out.push(vec!["admin".to_string()]);
    out.push(vec!["documents.write".to_string()]);
    // `screen.once` grants nothing now (#112) — it is the portal's
    // per-invocation scope, not a provenance-wide read.
    out.push(vec!["screen.once".to_string()]);
    out
}

/// Every (query, scopes) pair the gate exercises.
pub fn cases() -> Vec<(String, Vec<String>)> {
    let mut queries: Vec<String> = Vec::new();
    queries.extend(COLLIDING_TERMS.iter().map(|s| s.to_string()));
    queries.extend(PROVENANCE_UNIQUE_TERMS.iter().map(|(t, _)| t.to_string()));
    queries.extend(HOSTILE_QUERIES.iter().map(|s| s.to_string()));
    // Cross-products: every shared term paired with every term unique to
    // one provenance. These are the cases most likely to put a
    // disallowed chunk at the top of the ranking, because the shared half
    // matches everything and the unique half matches exactly one place.
    for a in COLLIDING_TERMS.iter() {
        for (t, _) in PROVENANCE_UNIQUE_TERMS {
            queries.push(format!("{a} {t}"));
            queries.push(format!("{a} OR {t}"));
        }
    }

    let scopes = scope_variants();
    let mut out = Vec::with_capacity(queries.len() * scopes.len());
    for q in &queries {
        for s in &scopes {
            out.push((q.clone(), s.clone()));
        }
    }
    out
}
