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
    pub fn prune_not_in(
        &self,
        provenance: &str,
        keep: &std::collections::HashSet<String>,
    ) -> Result<usize, StoreError> {
        if !is_known_provenance(provenance) {
            return Err(StoreError::UnknownProvenance(provenance.to_string()));
        }
        let conn = self.conn.lock().expect("context lock");
        let tx = conn.unchecked_transaction()?;
        let rows: Vec<(i64, String)> = tx
            .prepare("SELECT id, source FROM documents WHERE provenance = ?1")?
            .query_map([provenance], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut pruned = 0;
        for (doc_id, source) in rows {
            if keep.contains(&source) {
                continue;
            }
            tx.execute("DELETE FROM chunks WHERE doc_id = ?1", [doc_id])?;
            tx.execute("DELETE FROM chunk_vectors WHERE doc_id = ?1", [doc_id])?;
            tx.execute("DELETE FROM documents WHERE id = ?1", [doc_id])?;
            pruned += 1;
        }
        tx.commit()?;
        Ok(pruned)
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
