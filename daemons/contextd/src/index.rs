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
            let existing: Option<(i64, String)> = conn
                .query_row(
                    "SELECT id, content_hash FROM documents WHERE source = ?1",
                    [&source],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            if let Some((_, ref old_hash)) = existing
                && *old_hash == hash
            {
                report.skipped_unchanged += 1;
                continue;
            }
            if let Some((doc_id, _)) = existing {
                conn.execute("DELETE FROM chunks WHERE doc_id = ?1", [doc_id])?;
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
