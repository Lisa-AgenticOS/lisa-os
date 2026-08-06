//! Scoped, ACL-checked retrieval (`docs/PLAN.md` §5.3, §5.10). An app
//! granted `documents.read` must never receive a `mail` chunk — even if
//! it's the best hit. Retrieval enforces this at the query, mapping the
//! granted scopes to allowed provenance and filtering there, so a
//! disallowed chunk can't leak through ranking. The ACL fuzz suite (§5.3
//! acceptance: 0 cross-scope leaks) hammers this boundary.

use crate::index::Hit;
use crate::store::{ContextStore, StoreError};

/// Every provenance tag the store recognises.
///
/// A write with anything else is refused (issue #104). It used to be
/// accepted and then silently unreadable — every scope returned zero
/// hits for a document tagged `"File"` or `"clipboard"` — so a plugin
/// with a typo produced an invisible index rather than an error, and
/// nothing distinguished "indexed nothing" from "indexed into a tag
/// no scope can reach".
/// `system` is the OS knowledge pack (#175, ADR-0040): docs generated
/// at build time, describing the running image. Read-tier by design —
/// a doc chunk informs an answer, it never authorizes an action.
pub const PROVENANCE: [&str; 6] = ["file", "mail", "calendar", "screen", "web", "system"];

/// Whether `provenance` is one the ACL can reason about.
pub fn is_known_provenance(provenance: &str) -> bool {
    PROVENANCE.contains(&provenance)
}

/// Map a granted portal scope to the provenance tags it may read. Both
/// the portal scope names (`documents.read`) and their CLI short forms
/// (`documents`) resolve; an unknown scope grants nothing.
///
/// # `screen.once` is not here (issue #112)
///
/// It is the portal's *per-invocation* screen scope — PLAN §5.7.4:
/// screen context is on request only, with consent per capture and a
/// visible indicator while active. Mapping it to the whole `screen`
/// provenance turned one "share this window" into a durable read of
/// every capture ever pinned, which is a strictly wider grant than the
/// consent given, over the most sensitive class in the store — and
/// §5.7.4 explicitly refuses to build a Recall.
///
/// So it grants nothing here. A capability for the frames of the
/// current invocation is a different mechanism (a capture id in the
/// filter), and a durable historical read deserves its own scope name
/// so a consent dialog can say so — `screen.history`, which does not
/// exist yet and should not be invented before something needs it
/// (rule 8).
pub fn provenance_for_scope(scope: &str) -> &'static [&'static str] {
    match scope {
        "documents.read" | "files.read" | "documents" | "files" => &["file"],
        "mail.read" | "mail" => &["mail"],
        "calendar.read" | "calendar" => &["calendar"],
        // NOT "screen.once" — see above.
        "screen.read" | "screen" => &["screen"],
        "web.read" | "web" => &["web"],
        // The knowledge pack is world-readable ON THIS MACHINE by
        // definition — it ships in /usr/share. Every scope that can
        // read anything may also read it; a dedicated system.read
        // scope would be consent theater for public bytes.
        "system.read" | "system" => &["system"],
        _ => &[],
    }
}

/// What [`ContextStore::add_document_if_changed`] did with a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    Added,
    Unchanged,
    /// The source is already owned by a DIFFERENT provenance (#104):
    /// refused rather than relabeled, and counted so the caller can
    /// say so.
    ForeignSkipped,
}

/// All provenance tags the given scopes together permit.
pub fn allowed_provenance(scopes: &[&str]) -> Vec<&'static str> {
    let mut allowed: Vec<&'static str> = scopes
        .iter()
        .flat_map(|s| provenance_for_scope(s).iter().copied())
        .collect();
    allowed.sort_unstable();
    allowed.dedup();
    allowed
}

/// What a prune would remove, counted before anything is removed
/// (#224).
///
/// `vectors` is the number that matters: a chunk is cheap text, an
/// embedding is 3 KiB of float32 and the CPU that produced it. The
/// orphaned Maildir tree on the reference device was 26,117 of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunePlan {
    /// The sources that would go, in store order.
    pub sources: Vec<String>,
    pub chunks: usize,
    pub vectors: usize,
    /// Documents a `protect` prefix spared — unnamed by `keep` and
    /// therefore doomed under the mirror rule, but sitting under a
    /// corpus the caller has told us it could not read this time
    /// (#296). Zero when the caller protects nothing.
    pub protected: usize,
}

impl PrunePlan {
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

/// The documents of `provenance` that `keep` does not name and no
/// `protect` prefix covers, with what they own. One query serving both
/// the preview and the deletion, so the two cannot drift apart.
///
/// `protect` is the escape from the mirror rule's one bad assumption.
/// "Not named by `keep`" means "gone" only if the caller could actually
/// look; when it could not — a Maildir tree behind a symlink the walk
/// does not follow, a mount that is not up — every source under that
/// corpus is absent from `keep` for a reason that has nothing to do with
/// the mail (#296). A prefix says "I could not read this part", which is
/// a different sentence from "this part is empty", and only the caller
/// knows which one is true.
///
/// Counted with two grouped scans rather than a count per document: the
/// store this was written for holds 94,000 vectors and 43,000 mail
/// documents, and `chunks` is an FTS5 table whose `doc_id` is
/// `UNINDEXED` — a per-document `count(*)` there is a full scan each
/// time.
fn doomed(
    conn: &rusqlite::Connection,
    provenance: &str,
    keep: &std::collections::HashSet<String>,
    protect: &[String],
) -> Result<(Vec<i64>, PrunePlan), StoreError> {
    let tally = |sql: &str| -> Result<std::collections::HashMap<i64, usize>, StoreError> {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? as usize))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    };
    let chunks = tally("SELECT doc_id, count(*) FROM chunks GROUP BY doc_id")?;
    let vectors = tally("SELECT doc_id, count(*) FROM chunk_vectors GROUP BY doc_id")?;

    let rows: Vec<(i64, String)> = conn
        .prepare("SELECT id, source FROM documents WHERE provenance = ?1 ORDER BY id")?
        .query_map([provenance], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;

    let mut ids = Vec::new();
    let mut plan = PrunePlan::default();
    for (doc_id, source) in rows {
        if keep.contains(&source) {
            continue;
        }
        if protect.iter().any(|p| source.starts_with(p.as_str())) {
            plan.protected += 1;
            continue;
        }
        ids.push(doc_id);
        plan.sources.push(source);
        plan.chunks += chunks.get(&doc_id).copied().unwrap_or(0);
        plan.vectors += vectors.get(&doc_id).copied().unwrap_or(0);
    }
    Ok((ids, plan))
}

/// The one place a document's rows are written — inside the caller's
/// transaction, all three tables or none (#186).
fn write_document(
    tx: &rusqlite::Transaction,
    source: &str,
    provenance: &str,
    content: &str,
    hash: &str,
) -> Result<(), StoreError> {
    if let Ok(doc_id) = tx.query_row(
        "SELECT id FROM documents WHERE source = ?1",
        [source],
        |r| r.get::<_, i64>(0),
    ) {
        tx.execute("DELETE FROM chunks WHERE doc_id = ?1", [doc_id])?;
        tx.execute("DELETE FROM chunk_vectors WHERE doc_id = ?1", [doc_id])?;
        tx.execute("DELETE FROM documents WHERE id = ?1", [doc_id])?;
    }
    tx.execute(
        "INSERT INTO documents (source, provenance, mtime, content_hash)
         VALUES (?1, ?2, 0, ?3)",
        rusqlite::params![source, provenance, hash],
    )?;
    let doc_id = tx.last_insert_rowid();
    for (seq, chunk) in crate::index::chunk_text(content).iter().enumerate() {
        tx.execute(
            "INSERT INTO chunks (content, doc_id, seq) VALUES (?1, ?2, ?3)",
            rusqlite::params![chunk, doc_id, seq as i64],
        )?;
    }
    Ok(())
}

impl ContextStore {
    /// Incremental, relabel-safe ingestion — what a corpus mirror (the
    /// mail indexer, #170) calls per document.
    ///
    /// Unlike [`add_document`], this: skips unchanged content by hash
    /// (an indexer re-walking ten thousand messages must cost ten
    /// thousand hash compares, not ten thousand rewrites and
    /// re-embeds), and REFUSES to replace a document another
    /// provenance owns — the #104 rule, which add_document predates
    /// and does not enforce.
    pub fn add_document_if_changed(
        &self,
        source: &str,
        provenance: &str,
        content: &str,
    ) -> Result<AddOutcome, StoreError> {
        if !is_known_provenance(provenance) {
            return Err(StoreError::UnknownProvenance(provenance.to_string()));
        }
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        // ONE lock, ONE transaction (#186): the first version checked
        // the hash under a lock it then released, and wrote each row in
        // its own implicit transaction — so a crash mid-write left a
        // truncated document whose stored hash pinned it "Unchanged"
        // forever, and a concurrent timer + manual sync raced the
        // check into a UNIQUE(source) abort. BEGIN IMMEDIATE + the
        // busy_timeout in open() serialize writers instead.
        let conn = self.conn.lock().expect("context lock");
        let tx = conn.unchecked_transaction()?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT provenance, content_hash FROM documents WHERE source = ?1",
                [source],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        if let Some((ref old_provenance, ref old_hash)) = existing {
            if old_provenance != provenance {
                return Ok(AddOutcome::ForeignSkipped);
            }
            if *old_hash == hash {
                return Ok(AddOutcome::Unchanged);
            }
        }
        write_document(&tx, source, provenance, content, &hash)?;
        tx.commit()?;
        Ok(AddOutcome::Added)
    }

    /// Insert one document with an explicit provenance (mail/screen/web
    /// sources; files go through `index_dir`). Chunked + FTS-indexed.
    pub fn add_document(
        &self,
        source: &str,
        provenance: &str,
        content: &str,
    ) -> Result<(), StoreError> {
        if !is_known_provenance(provenance) {
            return Err(StoreError::UnknownProvenance(provenance.to_string()));
        }
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        let conn = self.conn.lock().expect("context lock");
        let tx = conn.unchecked_transaction()?;
        write_document(&tx, source, provenance, content, &hash)?;
        tx.commit()?;
        Ok(())
    }

    /// Every document of `provenance` whose source is NOT in `keep`
    /// is removed, chunks and vectors included. The mirror-pruning
    /// primitive for corpora whose source ids are not filesystem paths
    /// (mail's `mail:` namespace — `prune_missing` cannot see those,
    /// which is how deleted messages stayed retrievable forever, #185).
    ///
    /// Removes exactly what [`Self::plan_prune_not_in`] named, because
    /// both are the same query (`doomed`). A preview that is computed
    /// differently from the deletion is not a preview.
    ///
    /// `protect` names source prefixes the caller could not enumerate
    /// this run; nothing under one is removed. See [`doomed`].
    pub fn prune_not_in(
        &self,
        provenance: &str,
        keep: &std::collections::HashSet<String>,
        protect: &[String],
    ) -> Result<usize, StoreError> {
        if !is_known_provenance(provenance) {
            return Err(StoreError::UnknownProvenance(provenance.to_string()));
        }
        let conn = self.conn.lock().expect("context lock");
        let tx = conn.unchecked_transaction()?;
        let (ids, plan) = doomed(&tx, provenance, keep, protect)?;
        for doc_id in ids {
            tx.execute("DELETE FROM chunks WHERE doc_id = ?1", [doc_id])?;
            tx.execute("DELETE FROM chunk_vectors WHERE doc_id = ?1", [doc_id])?;
            tx.execute("DELETE FROM documents WHERE id = ?1", [doc_id])?;
        }
        tx.commit()?;
        Ok(plan.sources.len())
    }

    /// What [`Self::prune_not_in`] would remove, without removing it
    /// (#224).
    ///
    /// The reap this exists for is not hypothetical: on the reference
    /// device 9,094 mail documents and 26,117 embedding vectors — 27.7%
    /// of the whole store — belonged to a Maildir tree nothing syncs
    /// anymore. Deleting five figures of rows because a config file
    /// parsed one way rather than another is exactly the operation that
    /// should be printable before it happens.
    pub fn plan_prune_not_in(
        &self,
        provenance: &str,
        keep: &std::collections::HashSet<String>,
        protect: &[String],
    ) -> Result<PrunePlan, StoreError> {
        if !is_known_provenance(provenance) {
            return Err(StoreError::UnknownProvenance(provenance.to_string()));
        }
        let conn = self.conn.lock().expect("context lock");
        Ok(doomed(&conn, provenance, keep, protect)?.1)
    }

    /// Search restricted to the provenance the granted `scopes` permit.
    /// A chunk of disallowed provenance is never returned, regardless of
    /// its rank. Empty scopes → empty result (deny by default).
    pub fn search_scoped(
        &self,
        query: &str,
        scopes: &[&str],
        limit: usize,
    ) -> Result<Vec<Hit>, StoreError> {
        let allowed = allowed_provenance(scopes);
        if allowed.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", allowed.len())
            .collect::<Vec<_>>()
            .join(",");
        // All-anonymous placeholders bind positionally via
        // params_from_iter: query, provenance…, limit — no ?N/? mixing.
        let sql = format!(
            "SELECT d.source, d.provenance,
                    snippet(chunks, 0, '[', ']', ' … ', 12),
                    bm25(chunks)
             FROM chunks JOIN documents d ON d.id = chunks.doc_id
             WHERE chunks MATCH ? AND d.provenance IN ({placeholders})
             ORDER BY bm25(chunks) LIMIT ?"
        );
        // Same sanitising as the unscoped path: a question mark in the
        // user's prose is an FTS5 syntax error, not a search term.
        let Some(safe) = crate::index::fts5_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.conn.lock().expect("context lock");
        let mut stmt = conn.prepare(&sql)?;
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(safe)];
        for a in &allowed {
            params.push(Box::new(a.to_string()));
        }
        params.push(Box::new(limit as i64));
        let rows = stmt.query_map(
            rusqlite::params_from_iter(params.iter().map(|b| b.as_ref())),
            |r| {
                Ok(Hit {
                    source: r.get(0)?,
                    provenance: r.get(1)?,
                    snippet: r.get(2)?,
                    score: r.get(3)?,
                })
            },
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StoreError;

    /// #170's ingestion path: unchanged content is a no-op, changed
    /// content re-indexes, and a source another provenance owns is
    /// REFUSED (#104 — add_document predates that rule; the _if_changed
    /// variant enforces it).
    #[test]
    fn if_changed_skips_unchanged_and_refuses_relabels() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();

        assert_eq!(
            store
                .add_document_if_changed("mail:a/INBOX/1", "mail", "the parking permit renewal")
                .unwrap(),
            AddOutcome::Added
        );
        assert_eq!(
            store
                .add_document_if_changed("mail:a/INBOX/1", "mail", "the parking permit renewal")
                .unwrap(),
            AddOutcome::Unchanged
        );
        assert_eq!(
            store
                .add_document_if_changed("mail:a/INBOX/1", "mail", "edited body")
                .unwrap(),
            AddOutcome::Added
        );
        // A different provenance may not steal the source — refused,
        // and the original survives intact.
        assert_eq!(
            store
                .add_document_if_changed("mail:a/INBOX/1", "file", "impostor")
                .unwrap(),
            AddOutcome::ForeignSkipped
        );
        let hits = store.search_scoped("edited", &["mail"], 3).unwrap();
        assert!(
            !hits.is_empty(),
            "original mail doc must survive the impostor"
        );
        // And the scope wall holds: documents.read never sees mail.
        assert!(
            store
                .search_scoped("parking permit", &["documents"], 3)
                .unwrap()
                .iter()
                .all(|h| h.provenance != "mail"),
            "documents scope must not surface mail chunks"
        );
    }

    /// #224's cure primitive: the preview must name exactly what the
    /// deletion removes, vectors included — a plan computed by a
    /// different query from the delete is a plan that will one day be
    /// wrong about five figures of rows.
    #[test]
    fn the_prune_preview_is_the_prune() {
        use crate::embed::HashEmbedder;
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        for i in 0..4 {
            store
                .add_document(&format!("mail:INBOX/{i}"), "mail", "the parking permit")
                .unwrap();
        }
        store
            .add_document("/docs/a.md", "file", "unrelated")
            .unwrap();
        store.embed_pending(&HashEmbedder::default()).unwrap();

        let keep: std::collections::HashSet<String> =
            ["mail:INBOX/0".to_string(), "mail:INBOX/1".to_string()]
                .into_iter()
                .collect();
        let plan = store.plan_prune_not_in("mail", &keep, &[]).unwrap();
        assert_eq!(plan.sources, vec!["mail:INBOX/2", "mail:INBOX/3"]);
        assert!(plan.chunks >= 2, "{plan:?}");
        assert_eq!(plan.vectors, plan.chunks, "every chunk was embedded");
        assert_eq!(plan.protected, 0, "nothing was protected: {plan:?}");

        // Previewing removes nothing.
        assert_eq!(
            store.plan_prune_not_in("mail", &keep, &[]).unwrap(),
            plan,
            "the preview mutated the store"
        );
        let vectors = |store: &ContextStore| -> i64 {
            // The raw table, not a join: a delete that drops the
            // document and leaves its vectors satisfies every query
            // that goes through `documents`, and frees nothing.
            let conn = store.conn.lock().unwrap();
            conn.query_row("SELECT count(*) FROM chunk_vectors", [], |r| r.get(0))
                .unwrap()
        };
        let before = vectors(&store);
        assert_eq!(store.prune_not_in("mail", &keep, &[]).unwrap(), 2);
        assert_eq!(
            vectors(&store),
            before - plan.vectors as i64,
            "the reaped documents left their vectors in the table"
        );
        // …and afterwards there is nothing left to plan.
        assert!(
            store
                .plan_prune_not_in("mail", &keep, &[])
                .unwrap()
                .is_empty()
        );
        // The other provenance was never in scope.
        assert!(
            !store
                .search_scoped("unrelated", &["documents"], 3)
                .unwrap()
                .is_empty()
        );
    }

    /// #296's primitive: a prefix the caller could not enumerate is
    /// spared, and the prefix is a PREFIX of the whole path segment —
    /// `mail:b/` must not spare `mail:b2/…`, which is the classic way a
    /// string comparison quietly protects (or reaps) a neighbour.
    ///
    /// Also asserts the preview and the deletion agree under protection,
    /// because the whole point of `doomed` serving both is that a new
    /// parameter cannot be honoured by one and forgotten by the other.
    #[test]
    fn a_protected_prefix_is_spared_by_both_the_plan_and_the_prune() {
        use crate::embed::HashEmbedder;
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        for s in [
            "mail:a/INBOX/1",
            "mail:b/INBOX/1",
            "mail:b/INBOX/2",
            "mail:b2/INBOX/1",
        ] {
            store.add_document(s, "mail", "the parking permit").unwrap();
        }
        store.embed_pending(&HashEmbedder::default()).unwrap();

        // `keep` names nothing at all — the shape of a walk that saw no
        // messages. Only the protection stands between b/ and deletion.
        let keep = std::collections::HashSet::new();
        let protect = vec!["mail:b/".to_string()];
        let plan = store.plan_prune_not_in("mail", &keep, &protect).unwrap();
        assert_eq!(
            plan.sources,
            vec!["mail:a/INBOX/1", "mail:b2/INBOX/1"],
            "the protected prefix ate a neighbour, or spared nothing: {plan:?}"
        );
        assert_eq!(plan.protected, 2, "{plan:?}");
        assert!(plan.vectors > 0, "{plan:?}");

        assert_eq!(store.prune_not_in("mail", &keep, &protect).unwrap(), 2);
        let left: Vec<String> = {
            let conn = store.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT source FROM documents WHERE provenance = 'mail' ORDER BY source")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get(0)).unwrap();
            rows.collect::<Result<_, _>>().unwrap()
        };
        assert_eq!(
            left,
            vec!["mail:b/INBOX/1", "mail:b/INBOX/2"],
            "the prune disagreed with its own plan"
        );
        // The spared documents kept their vectors: sparing a document
        // and orphaning its embeddings is the cost the reap exists to
        // avoid, paid in the other direction.
        let conn = store.conn.lock().unwrap();
        let vectors: i64 = conn
            .query_row("SELECT count(*) FROM chunk_vectors", [], |r| r.get(0))
            .unwrap();
        assert!(vectors > 0, "the protected documents lost their vectors");
    }

    fn mixed_store() -> (tempfile::TempDir, ContextStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        store
            .add_document(
                "/docs/report.md",
                "file",
                "quarterly revenue report: budget and forecast numbers",
            )
            .unwrap();
        store
            .add_document(
                "mail://inbox/42",
                "mail",
                "Re: budget — the revenue forecast looks off, can we talk",
            )
            .unwrap();
        store
            .add_document(
                "screen://capture/1",
                "screen",
                "spreadsheet showing budget revenue forecast on screen",
            )
            .unwrap();
        (dir, store)
    }

    /// Issue #104's second half: a document written under a tag no
    /// scope maps to is readable by nobody, which looks exactly like an
    /// empty index. A typo in a plugin should be an error, not an
    /// invisible corpus.
    #[test]
    fn an_unknown_provenance_is_refused_rather_than_silently_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        for bad in ["", "File", "clipboard", "files", "SCREEN"] {
            assert!(
                matches!(
                    store.add_document("s", bad, "content"),
                    Err(StoreError::UnknownProvenance(_))
                ),
                "{bad:?} was accepted"
            );
        }
        for good in PROVENANCE {
            store
                .add_document(&format!("s-{good}"), good, "content")
                .unwrap_or_else(|e| panic!("{good} was refused: {e}"));
        }
    }

    /// Issue #112. `screen.once` is the portal's per-invocation scope —
    /// one "share this window". It used to map to the whole `screen`
    /// provenance, so that single consent read every capture ever
    /// pinned into the index.
    #[test]
    fn screen_once_does_not_grant_the_whole_capture_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        store
            .add_document("cap-1", "screen", "a password on somebody screen")
            .unwrap();
        store
            .add_document("cap-2", "screen", "another screen with a password")
            .unwrap();

        assert!(
            store
                .search_scoped("password", &["screen.once"], 10)
                .unwrap()
                .is_empty(),
            "screen.once read the capture history"
        );
        assert!(provenance_for_scope("screen.once").is_empty());

        // The durable scope still works — this is a narrowing of one
        // scope, not the removal of screen retrieval.
        assert_eq!(
            store
                .search_scoped("password", &["screen.read"], 10)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn documents_scope_never_returns_mail_or_screen() {
        let (_dir, store) = mixed_store();
        // "budget revenue forecast" matches ALL three provenances; the
        // mail chunk may even rank best. Scope must still exclude it.
        let hits = store
            .search_scoped("budget revenue forecast", &["documents.read"], 10)
            .unwrap();
        assert!(!hits.is_empty(), "the file doc should match");
        assert!(
            hits.iter().all(|h| h.provenance == "file"),
            "cross-scope leak: {hits:?}"
        );
    }

    #[test]
    fn empty_scopes_deny_by_default() {
        let (_dir, store) = mixed_store();
        assert!(store.search_scoped("budget", &[], 10).unwrap().is_empty());
        assert!(
            store
                .search_scoped("budget", &["inference"], 10)
                .unwrap()
                .is_empty(),
            "an unrelated scope grants no provenance"
        );
    }

    #[test]
    fn acl_fuzz_zero_cross_scope_leaks() {
        let (_dir, store) = mixed_store();
        // Every scope only ever yields its own provenance, across many
        // query shapes — the §5.3 "0 cross-scope leaks" acceptance in
        // miniature, kept here so a change to this file fails fast. The
        // full suite (15,618 cases, hostile FTS/SQL query shapes, scope
        // spelling variants, and a non-vacuity floor) is tests/acl-fuzz.
        let queries = [
            "budget",
            "revenue",
            "forecast",
            "report",
            "numbers",
            "talk",
            "spreadsheet",
            "quarterly",
            "off",
            "budget revenue",
            "the",
        ];
        let cases = [
            ("documents.read", "file"),
            ("mail.read", "mail"),
            ("screen.once", "screen"),
        ];
        for q in queries {
            for (scope, provenance) in cases {
                for h in store.search_scoped(q, &[scope], 10).unwrap() {
                    assert_eq!(h.provenance, provenance, "leak: {scope} returned {h:?}");
                }
            }
        }
    }
}
