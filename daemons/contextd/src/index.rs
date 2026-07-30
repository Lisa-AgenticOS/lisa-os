//! File ingestion + lexical retrieval (`docs/PLAN.md` §5.3).
//!
//! Pipeline v0: walk → filter to text-like files → chunk (~1 KiB on
//! paragraph boundaries) → FTS5. Incremental: unchanged (mtime + hash)
//! documents are skipped; changed ones are reindexed atomically.
//! Embeddings (via inferenced, background QoS) and hybrid ranking join
//! this in the next pass; retrieval already returns provenance.

use crate::store::{ContextStore, StoreError};
use std::path::Path;
use walkdir::WalkDir;

const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "rst", "org", "html", "htm", "json", "toml", "yaml", "yml", "csv", "log", "rs",
    "py", "js", "ts", "sh", "c", "h", "cpp", "go", "java",
];
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const CHUNK_TARGET: usize = 1024;

#[derive(Debug, Default)]
pub struct IndexReport {
    pub indexed: usize,
    pub skipped_unchanged: usize,
    pub chunks: usize,
    /// Paths that are already indexed under a non-`file` provenance —
    /// a mail or screen document whose source id happens to be a path
    /// inside the walked tree (issue #104). Named, not just counted:
    /// "the walker declined to touch your Maildir" is something the
    /// person running it should be able to read.
    pub skipped_foreign: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Hit {
    pub source: String,
    pub provenance: String,
    pub snippet: String,
    /// FTS5 bm25 — lower ranks better.
    pub score: f64,
}

/// Split on blank-line paragraph boundaries, packing to ~CHUNK_TARGET.
pub fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for para in text.split("\n\n") {
        if !current.is_empty() && current.len() + para.len() > CHUNK_TARGET {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
        // A single oversized paragraph still becomes its own chunk(s).
        while current.len() > CHUNK_TARGET * 2 {
            let split_at = current
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= CHUNK_TARGET * 2)
                .last()
                .unwrap_or(current.len());
            let rest = current.split_off(split_at);
            chunks.push(std::mem::take(&mut current));
            current = rest;
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
}

impl ContextStore {
    /// Index every text-like file under `root`. Returns what changed.
    pub fn index_dir(&self, root: &Path) -> Result<IndexReport, StoreError> {
        let mut report = IndexReport::default();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !TEXT_EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path) else {
                continue; // non-UTF8 or unreadable: extraction lands later
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
            let source = path.to_string_lossy().into_owned();

            let conn = self.conn.lock().expect("context lock");
            let existing: Option<(i64, String, String)> = conn
                .query_row(
                    "SELECT id, content_hash, provenance FROM documents WHERE source = ?1",
                    [&source],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .ok();
            // A foreign row is skipped whether or not its content
            // happens to match — otherwise the hash decides the label,
            // which is the instability described below.
            if let Some((_, _, ref old_provenance)) = existing
                && old_provenance != "file"
            {
                report.skipped_foreign.push(source.clone());
                continue;
            }
            if let Some((_, ref old_hash, _)) = existing
                && *old_hash == hash
            {
                report.skipped_unchanged += 1;
                continue;
            }
            if let Some((doc_id, _, ref old_provenance)) = existing {
                // Never relabel somebody else's document (issue #104).
                //
                // This keyed only on `source`, and then re-inserted with
                // a hardcoded `'file'`. The natural source id for the
                // planned mail plugin *is* a filesystem path — notmuch
                // and Maildir messages are files, under $HOME, inside
                // exactly the tree this walker covers — so a mail
                // document silently became `file` provenance and
                // `documents.read` could read it.
                //
                // Worse, it was content-dependent: if the bytes on disk
                // happened to hash equal to the stored hash the
                // skipped_unchanged branch above kept the old label. The
                // same corpus classified differently depending on
                // whether a plugin's extracted text matched the file's.
                //
                // Skipped rather than fatal: one such collision must not
                // abort the walk over an entire home directory. It is
                // counted and named so it is visible.
                if old_provenance != "file" {
                    report.skipped_foreign.push(source.clone());
                    continue;
                }
                conn.execute("DELETE FROM chunks WHERE doc_id = ?1", [doc_id])?;
                // The third table — missed here while `add_document`
                // deleted all three (issue #105). Every reindex left the
                // old embeddings behind: residual content that survives
                // a "reindex", and a table that grows without bound.
                conn.execute("DELETE FROM chunk_vectors WHERE doc_id = ?1", [doc_id])?;
                conn.execute("DELETE FROM documents WHERE id = ?1", [doc_id])?;
            }
            conn.execute(
                "INSERT INTO documents (source, provenance, mtime, content_hash)
                 VALUES (?1, 'file', ?2, ?3)",
                rusqlite::params![source, mtime, hash],
            )?;
            let doc_id = conn.last_insert_rowid();
            for (seq, chunk) in chunk_text(&content).iter().enumerate() {
                conn.execute(
                    "INSERT INTO chunks (content, doc_id, seq) VALUES (?1, ?2, ?3)",
                    rusqlite::params![chunk, doc_id, seq as i64],
                )?;
                report.chunks += 1;
            }
            report.indexed += 1;
        }
        Ok(report)
    }

    /// Lexical retrieval (FTS5 bm25), best first, with provenance.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>, StoreError> {
        let conn = self.conn.lock().expect("context lock");
        let mut stmt = conn.prepare(
            "SELECT d.source, d.provenance,
                    snippet(chunks, 0, '[', ']', ' … ', 12),
                    bm25(chunks)
             FROM chunks JOIN documents d ON d.id = chunks.doc_id
             WHERE chunks MATCH ?1
             ORDER BY bm25(chunks) LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![query, limit as i64], |r| {
            Ok(Hit {
                source: r.get(0)?,
                provenance: r.get(1)?,
                snippet: r.get(2)?,
                score: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_pack_paragraphs_to_target() {
        let text = (0..40)
            .map(|i| format!("paragraph {i} with some words in it"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let chunks = chunk_text(&text);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.len() <= CHUNK_TARGET * 2 + 64));
    }

    #[test]
    fn index_and_retrieve_the_planted_document() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("biology.md"),
            "# Notes\n\nThe mitochondria is the powerhouse of the cell.\n\nOther text.",
        )
        .unwrap();
        std::fs::write(dir.path().join("cooking.md"), "How to boil pasta properly.").unwrap();
        std::fs::write(dir.path().join("image.png"), [0u8, 1, 2]).unwrap();

        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        let report = store.index_dir(dir.path()).unwrap();
        assert_eq!(report.indexed, 2, "png must be skipped");

        let hits = store.search("mitochondria", 3).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].source.ends_with("biology.md"));
        assert_eq!(hits[0].provenance, "file");
        assert!(hits[0].snippet.contains("[mitochondria]"));
    }

    #[test]
    fn reindex_skips_unchanged_and_updates_changed() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("note.txt");
        std::fs::write(&f, "original alpaca content").unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();

        let r1 = store.index_dir(dir.path()).unwrap();
        assert_eq!((r1.indexed, r1.skipped_unchanged), (1, 0));

        let r2 = store.index_dir(dir.path()).unwrap();
        assert_eq!((r2.indexed, r2.skipped_unchanged), (0, 1));

        std::fs::write(&f, "replaced zebra content").unwrap();
        let r3 = store.index_dir(dir.path()).unwrap();
        assert_eq!(r3.indexed, 1);
        assert!(store.search("alpaca", 3).unwrap().is_empty(), "stale chunk");
        assert!(!store.search("zebra", 3).unwrap().is_empty());
    }
}

#[cfg(test)]
mod reindex_tests {
    use crate::store::ContextStore;

    /// Issue #104, as filed. A plugin's source id is a filesystem path
    /// inside the tree the walker covers — an exported message, a web
    /// or screen cache file — so the document was relabelled `file` on
    /// the next index and `documents.read` could read it.
    ///
    /// The extension matters: `index_dir` only looks at TEXT_EXTENSIONS,
    /// so a bare Maildir name (`1234.M567P890.host:2,S`) is skipped
    /// before provenance is ever considered. Written first with a
    /// `.mail` suffix, which passed with the fix reverted — the walker
    /// had simply never reached it. `.txt` is a path the walker really
    /// does take.
    #[test]
    fn indexing_never_relabels_a_foreign_document_as_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = tempfile::tempdir().unwrap();
        let store = ContextStore::open(db.path().join("ctx.db")).unwrap();

        // A message exported to a text file inside the indexed tree.
        let path = dir.path().join("cur").join("1234.txt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "mail-ish secret content sauce").unwrap();
        let source = path.to_string_lossy().into_owned();
        store
            .add_document(&source, "mail", "mail-ish secret content sauce")
            .unwrap();
        assert!(
            store
                .search_scoped("sauce", &["documents.read"], 50)
                .unwrap()
                .is_empty(),
            "precondition: documents.read cannot read mail"
        );

        let report = store.index_dir(dir.path()).unwrap();

        assert!(
            store
                .search_scoped("sauce", &["documents.read"], 50)
                .unwrap()
                .is_empty(),
            "the mail document was relabelled and is now documents.read-readable"
        );
        assert_eq!(
            store
                .search_scoped("sauce", &["mail.read"], 50)
                .unwrap()
                .len(),
            1,
            "the mail document should still be readable as mail"
        );
        assert_eq!(
            report.skipped_foreign,
            vec![source],
            "the skip should be named, not silent"
        );
    }

    /// The same, when the bytes on disk happen to match what the plugin
    /// stored. This was the unstable half: an equal hash took the
    /// skipped_unchanged branch and kept the old label, so the same
    /// corpus classified differently depending on the file's contents.
    /// Both paths must reach the same answer.
    #[test]
    fn the_decision_does_not_depend_on_whether_the_bytes_match() {
        let dir = tempfile::tempdir().unwrap();
        let db = tempfile::tempdir().unwrap();
        let store = ContextStore::open(db.path().join("ctx.db")).unwrap();
        let path = dir.path().join("note.txt");

        // Case A: stored text differs from the file's.
        std::fs::write(&path, "on-disk text").unwrap();
        let source = path.to_string_lossy().into_owned();
        store
            .add_document(&source, "mail", "different text")
            .unwrap();
        store.index_dir(dir.path()).unwrap();
        let a = store
            .search_scoped("text", &["mail.read"], 50)
            .unwrap()
            .len();

        // Case B: identical.
        let db2 = tempfile::tempdir().unwrap();
        let store2 = ContextStore::open(db2.path().join("ctx.db")).unwrap();
        store2
            .add_document(&source, "mail", "on-disk text")
            .unwrap();
        store2.index_dir(dir.path()).unwrap();
        let b = store2
            .search_scoped("text", &["mail.read"], 50)
            .unwrap()
            .len();

        assert_eq!(a, b, "provenance depended on whether the content matched");
        assert_eq!(a, 1, "the document should have stayed mail in both cases");
    }

    /// Issue #105: `index_dir` deleted chunks and documents but not
    /// `chunk_vectors`, so every reindex left the previous embeddings
    /// behind — residual content that survives a reindex, in a table
    /// that grows without bound.
    #[test]
    fn reindexing_leaves_no_orphaned_vectors() {
        let dir = tempfile::tempdir().unwrap();
        let db = tempfile::tempdir().unwrap();
        let store = ContextStore::open(db.path().join("ctx.db")).unwrap();
        let path = dir.path().join("note.txt");

        let orphans = |s: &ContextStore| -> i64 {
            let conn = s.conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM chunk_vectors v
                 WHERE NOT EXISTS (SELECT 1 FROM documents d WHERE d.id = v.doc_id)",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };

        for i in 0..4 {
            std::fs::write(&path, format!("revision {i} of the note")).unwrap();
            store.index_dir(dir.path()).unwrap();
            store
                .embed_pending(&crate::embed::HashEmbedder::default())
                .unwrap();
        }
        assert_eq!(
            orphans(&store),
            0,
            "reindexing left orphaned vectors behind"
        );
    }
}
