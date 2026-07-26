//! Scoped, ACL-checked retrieval (`docs/PLAN.md` §5.3, §5.10). An app
//! granted `documents.read` must never receive a `mail` chunk — even if
//! it's the best hit. Retrieval enforces this at the query, mapping the
//! granted scopes to allowed provenance and filtering there, so a
//! disallowed chunk can't leak through ranking. The ACL fuzz suite (§5.3
//! acceptance: 0 cross-scope leaks) hammers this boundary.

use crate::index::Hit;
use crate::store::{ContextStore, StoreError};

/// Map a granted portal scope to the provenance tags it may read. Both
/// the portal scope names (`documents.read`) and their CLI short forms
/// (`documents`) resolve; an unknown scope grants nothing.
pub fn provenance_for_scope(scope: &str) -> &'static [&'static str] {
    match scope {
        "documents.read" | "files.read" | "documents" | "files" => &["file"],
        "mail.read" | "mail" => &["mail"],
        "calendar.read" | "calendar" => &["calendar"],
        "screen.once" | "screen.read" | "screen" => &["screen"],
        "web.read" | "web" => &["web"],
        _ => &[],
    }
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

impl ContextStore {
    /// Insert one document with an explicit provenance (mail/screen/web
    /// sources; files go through `index_dir`). Chunked + FTS-indexed.
    pub fn add_document(
        &self,
        source: &str,
        provenance: &str,
        content: &str,
    ) -> Result<(), StoreError> {
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        let conn = self.conn.lock().expect("context lock");
        // Replace any prior version of this source.
        if let Ok(doc_id) = conn.query_row(
            "SELECT id FROM documents WHERE source = ?1",
            [source],
            |r| r.get::<_, i64>(0),
        ) {
            conn.execute("DELETE FROM chunks WHERE doc_id = ?1", [doc_id])?;
            conn.execute("DELETE FROM chunk_vectors WHERE doc_id = ?1", [doc_id])?;
            conn.execute("DELETE FROM documents WHERE id = ?1", [doc_id])?;
        }
        conn.execute(
            "INSERT INTO documents (source, provenance, mtime, content_hash)
             VALUES (?1, ?2, 0, ?3)",
            rusqlite::params![source, provenance, hash],
        )?;
        let doc_id = conn.last_insert_rowid();
        for (seq, chunk) in crate::index::chunk_text(content).iter().enumerate() {
            conn.execute(
                "INSERT INTO chunks (content, doc_id, seq) VALUES (?1, ?2, ?3)",
                rusqlite::params![chunk, doc_id, seq as i64],
            )?;
        }
        Ok(())
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
        let conn = self.conn.lock().expect("context lock");
        let mut stmt = conn.prepare(&sql)?;
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(query.to_string())];
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
