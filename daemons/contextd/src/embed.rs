//! Embedding pipeline + hybrid retrieval (`docs/PLAN.md` §5.3).
//!
//! Retrieval that flat lexical search can't do: embed each chunk, embed
//! the query, and rank by a blend of BM25 (lexical) and cosine (vector).
//! Embedding is pluggable via [`Embedder`].
//!
//! WHAT ACTUALLY EMBEDS TODAY: [`HashEmbedder`], everywhere — the
//! daemon's D-Bus paths and the CLI both construct it directly. The
//! model-backed embedder (a background-QoS call to `lisa-inferenced`'s
//! `/v1/embeddings`, loopback, not egress) is the DESIGN and does not
//! exist yet (#163); the one real `/v1/embeddings` call in the tree is
//! the standalone `lisa embed` verb, which feeds nothing here. An
//! earlier version of this header described that wiring as current
//! behavior, which is this repo's most repeated defect (CLAUDE.md
//! rule 10) — hybrid=true returned hits in a different order and
//! nothing could tell there was no semantic model behind it.
//!
//! Vectors persist in `chunk_vectors`; ranking is brute-force cosine
//! over the FTS5-prefiltered candidate set (sqlite-vec is the later
//! optimization at >5M chunks, PLAN §13).

use crate::index::Hit;
use crate::store::{ContextStore, StoreError};

/// Turns texts into vectors. Impls must be deterministic per text so a
/// re-index doesn't churn the store.
pub trait Embedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, StoreError>;
}

/// Deterministic bag-of-words hashing embedder. Similar texts (shared
/// tokens) get similar vectors, so it exercises the hybrid path without
/// a model. Not for production quality — a real model plugs in via the
/// same trait — but honest for tests and an offline fallback.
pub struct HashEmbedder {
    pub dim: usize,
}

impl Default for HashEmbedder {
    fn default() -> Self {
        Self { dim: 64 }
    }
}

impl Embedder for HashEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, StoreError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim];
                for token in t.split(|c: char| !c.is_alphanumeric()) {
                    if token.is_empty() {
                        continue;
                    }
                    let lower = token.to_ascii_lowercase();
                    let mut h: u64 = 1469598103934665603;
                    for b in lower.bytes() {
                        h = (h ^ u64::from(b)).wrapping_mul(1099511628211);
                    }
                    v[(h as usize) % self.dim] += 1.0;
                }
                normalize(&mut v);
                v
            })
            .collect())
    }
}

fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    // Vectors are stored normalized, so cosine is the dot product.
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

impl ContextStore {
    /// Embed every indexed chunk that doesn't yet have a vector. Runs
    /// after `index_dir` (incremental — re-runs only touch new chunks).
    /// Returns how many chunks were embedded.
    pub fn embed_pending(&self, embedder: &dyn Embedder) -> Result<usize, StoreError> {
        let conn = self.conn.lock().expect("context lock");
        let mut stmt = conn.prepare(
            "SELECT c.doc_id, c.seq, c.content
             FROM chunks c
             LEFT JOIN chunk_vectors v ON v.doc_id = c.doc_id AND v.seq = c.seq
             WHERE v.doc_id IS NULL",
        )?;
        let pending: Vec<(i64, i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        drop(stmt);
        if pending.is_empty() {
            return Ok(0);
        }
        let texts: Vec<String> = pending.iter().map(|(_, _, c)| c.clone()).collect();
        let vectors = embedder.embed(&texts)?;
        for ((doc_id, seq, _), vec) in pending.iter().zip(vectors.iter()) {
            conn.execute(
                "INSERT OR REPLACE INTO chunk_vectors (doc_id, seq, dim, vec)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![doc_id, seq, vec.len() as i64, vec_to_blob(vec)],
            )?;
        }
        Ok(pending.len())
    }

    /// Hybrid search: FTS5 (BM25) prefilter → cosine rerank → blended
    /// score. Chunks the query would miss lexically but match
    /// semantically still surface if a lexical candidate shares the
    /// vector neighborhood. Falls back to lexical order when a candidate
    /// has no vector yet.
    pub fn search_hybrid(
        &self,
        query: &str,
        embedder: &dyn Embedder,
        limit: usize,
    ) -> Result<Vec<Hit>, StoreError> {
        // Pull a generous lexical candidate set to rerank.
        let candidates = self.search(query, limit.max(20) * 3)?;
        self.rerank(query, candidates, embedder, limit)
    }

    /// The same blend, over an ACL-filtered candidate set.
    ///
    /// Hybrid is a *ranking* choice and the ACL is a *visibility*
    /// choice; coupling them meant an app that asked for both got
    /// whichever the code happened to branch on. Reranking the scoped
    /// candidates keeps them orthogonal — a disallowed chunk is never a
    /// candidate, so it cannot be reranked into the answer either.
    pub fn search_hybrid_scoped(
        &self,
        query: &str,
        scopes: &[&str],
        embedder: &dyn Embedder,
        limit: usize,
    ) -> Result<Vec<Hit>, StoreError> {
        let candidates = self.search_scoped(query, scopes, limit.max(20) * 3)?;
        self.rerank(query, candidates, embedder, limit)
    }

    fn rerank(
        &self,
        query: &str,
        candidates: Vec<Hit>,
        embedder: &dyn Embedder,
        limit: usize,
    ) -> Result<Vec<Hit>, StoreError> {
        if candidates.is_empty() {
            return Ok(candidates);
        }
        let qvec = embedder.embed(std::slice::from_ref(&query.to_string()))?;
        let qvec = &qvec[0];

        let conn = self.conn.lock().expect("context lock");
        // Best BM25 magnitude for normalization (bm25 is negative; more
        // negative = better).
        let best_bm25 = candidates
            .iter()
            .map(|c| c.score)
            .fold(f64::INFINITY, f64::min)
            .abs()
            .max(1e-6);

        let mut scored: Vec<(f64, Hit)> = Vec::with_capacity(candidates.len());
        for hit in candidates {
            // Look up this hit's best chunk vector by source.
            let vec: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT v.vec FROM chunk_vectors v
                     JOIN documents d ON d.id = v.doc_id
                     WHERE d.source = ?1
                     ORDER BY v.seq LIMIT 1",
                    [&hit.source],
                    |r| r.get(0),
                )
                .ok();
            let cos = vec
                .map(|b| cosine(qvec, &blob_to_vec(&b)) as f64)
                .unwrap_or(0.0);
            let lex = hit.score.abs() / best_bm25; // 0..1, higher better
            let blended = 0.5 * lex + 0.5 * ((cos + 1.0) / 2.0);
            scored.push((blended, hit));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored.into_iter().take(limit).map(|(_, h)| h).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_search_embeds_and_reranks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("cell.md"),
            "The mitochondria is the powerhouse of the cell, producing energy.",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("kitchen.md"),
            "The oven is the powerhouse of the kitchen for baking bread.",
        )
        .unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        store.index_dir(dir.path()).unwrap();

        let embedder = HashEmbedder::default();
        let embedded = store.embed_pending(&embedder).unwrap();
        assert!(embedded >= 2, "both docs embedded");
        // Re-run is a no-op (incremental).
        assert_eq!(store.embed_pending(&embedder).unwrap(), 0);

        // "cell energy" leans toward the biology doc via the vector blend.
        let hits = store.search_hybrid("cell energy", &embedder, 2).unwrap();
        assert!(!hits.is_empty());
        assert!(
            hits[0].source.ends_with("cell.md"),
            "hybrid should rank the biology chunk first: {hits:?}"
        );
    }

    #[test]
    fn hybrid_falls_back_gracefully_without_vectors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "quantum entanglement notes").unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        store.index_dir(dir.path()).unwrap();
        // No embed_pending called → no vectors; hybrid still returns
        // lexical hits.
        let hits = store
            .search_hybrid("quantum", &HashEmbedder::default(), 5)
            .unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].source.ends_with("a.md"));
    }
}
