//! Embedding pipeline + hybrid retrieval (`docs/PLAN.md` §5.3).
//!
//! Retrieval that flat lexical search can't do: embed each chunk, embed
//! the query, and rank by a blend of BM25 (lexical) and cosine (vector).
//! Embedding is pluggable via [`Embedder`].
//!
//! WHAT ACTUALLY EMBEDS TODAY: whichever [`resolve`] returns, and it
//! says which. [`InferencedEmbedder`] when `lisa-inferenced`'s unix
//! socket answers, [`HashEmbedder`] otherwise — and the fallback is
//! never quiet: it logs a warning, the CLI prints a note, and the
//! Ledger entry for the search carries `"embedder": "hash"`. That is
//! the whole of #163: `hybrid=true` used to return plausibly-ranked
//! hits with no semantic model behind them, and neither a caller nor a
//! reviewer could tell.
//!
//! The socket, not `127.0.0.1:7777`, because this daemon runs with
//! `RestrictAddressFamilies=AF_UNIX` and `IPAddressDeny=any` — see
//! [`InferencedEmbedder`] for why that shapes the design rather than
//! being worked around.
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

/// The model-backed embedder: `lisa-inferenced`'s `/v1/embeddings`,
/// reached over a **unix socket** rather than `127.0.0.1:7777`.
///
/// The socket is not a stylistic preference. This daemon runs with
/// `RestrictAddressFamilies=AF_UNIX` and `IPAddressDeny=any`
/// (`os/packages/lisa/lisa-contextd-user.service`) — it cannot open an
/// IP socket at all, loopback included, because "contextd never reaches
/// the network" is a kernel-enforced property and not a promise
/// (CLAUDE.md rule 5). An embedder written against the loopback port
/// would compile, pass every test on a dev host where no unit file
/// applies, and fail only on the device, silently, as a fallback to
/// [`HashEmbedder`] — which is the exact defect #163 was filed about.
pub struct InferencedEmbedder {
    socket: std::path::PathBuf,
    model: Option<String>,
}

/// The model that does embeddings, by name.
///
/// This mirrors the single `task = "embeddings"` entry in
/// `models/catalog/catalog.toml`, which is the project's source of truth
/// for what each model is for. Duplicating the id here is deliberate —
/// the catalog is build-time data that contextd does not read at runtime
/// — and `os/repo-tools/check-embedding-model.py` fails the lint if the
/// two ever disagree, so the duplicate cannot rot quietly.
pub const EMBEDDING_MODEL: &str = "nomic-embed-text-v1.5";

impl InferencedEmbedder {
    /// Where the per-user companion puts its socket.
    pub fn default_socket() -> Option<std::path::PathBuf> {
        std::env::var_os("LISA_INFERENCED_SOCKET")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_RUNTIME_DIR")
                    .map(std::path::PathBuf::from)
                    .map(|d| d.join("lisa/inferenced.sock"))
            })
    }

    pub fn new(socket: std::path::PathBuf, model: Option<String>) -> Self {
        Self { socket, model }
    }

    /// Which model this embedder will name in its requests, if any.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Ask the daemon for [`EMBEDDING_MODEL`] only if it actually has it.
    ///
    /// Naming a model the daemon has not loaded is worse than naming
    /// none: `engine_for` fails the request outright, so a device
    /// without the embedding model downloaded would go from "embeds with
    /// a chat model, adequately" to "cannot embed at all". Asking
    /// `/v1/models` first is what keeps the improvement from being a
    /// regression on the machine that has not run `lisa models get`.
    pub fn preferred_model(socket: &std::path::Path) -> Option<String> {
        let raw = http_get(socket, "/v1/models").ok()?;
        let json: serde_json::Value = serde_json::from_slice(&raw).ok()?;
        json["data"]
            .as_array()?
            .iter()
            .filter_map(|m| m["id"].as_str())
            .find(|id| *id == EMBEDDING_MODEL)
            .map(str::to_string)
    }
}

/// Minimal HTTP/1.1 GET over a unix socket, returning the body.
fn http_get(socket: &std::path::Path, path: &str) -> Result<Vec<u8>, StoreError> {
    use std::io::{Read, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
    let head = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nAccept: */*\r\n\r\n"
    );
    stream.write_all(head.as_bytes())?;
    stream.flush()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| io_msg("no header/body separator in the response"))?;
    let status = String::from_utf8_lossy(&raw[..split])
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("?")
        .to_string();
    if status != "200" {
        return Err(io_msg(&format!("GET {path} returned HTTP {status}")));
    }
    Ok(raw[split + 4..].to_vec())
}

impl Embedder for InferencedEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, StoreError> {
        use std::io::{Read, Write};
        let mut body = serde_json::json!({ "input": texts });
        if let Some(model) = &self.model {
            body["model"] = serde_json::Value::String(model.clone());
        }
        let body = serde_json::to_vec(&body).map_err(io_err)?;
        let mut stream = std::os::unix::net::UnixStream::connect(&self.socket)?;
        // `Connection: close` is what makes read_to_end the whole
        // response: the server hangs up when it is done, so there is no
        // framing to get wrong and no keep-alive state to manage. Host
        // is required by HTTP/1.1 and ignored over a socket.
        let head = format!(
            "POST /v1/embeddings HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(&body)?;
        stream.flush()?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw)?;

        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| io_msg("no header/body separator in the embeddings response"))?;
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let payload = &raw[split + 4..];
        let status = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("?")
            .to_string();
        // Refuse rather than mis-parse. Nothing this daemon asks for
        // should come back chunked (a JSON body has a known length), so
        // if one does, the assumption above has stopped holding and
        // silently reading framing bytes as JSON would be worse.
        if head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            return Err(io_msg(
                "embeddings response is chunked; this client reads Content-Length bodies only",
            ));
        }
        if status != "200" {
            return Err(io_msg(&format!(
                "embeddings returned HTTP {status}: {}",
                String::from_utf8_lossy(payload).trim()
            )));
        }
        let json: serde_json::Value = serde_json::from_slice(payload).map_err(io_err)?;
        if let Some(msg) = json["error"]["message"].as_str() {
            return Err(io_msg(&format!("embeddings error: {msg}")));
        }
        let data = json["data"]
            .as_array()
            .ok_or_else(|| io_msg("embeddings response has no data array"))?;
        // One vector per input, in order. A short reply would silently
        // misalign vectors with chunks — every document after the gap
        // would carry someone else's meaning.
        if data.len() != texts.len() {
            return Err(io_msg(&format!(
                "embeddings returned {} vectors for {} inputs",
                data.len(),
                texts.len()
            )));
        }
        let mut out = Vec::with_capacity(data.len());
        for item in data {
            let v = item["embedding"]
                .as_array()
                .ok_or_else(|| io_msg("an embeddings entry has no embedding array"))?;
            let mut v: Vec<f32> = v
                .iter()
                .filter_map(|x| x.as_f64())
                .map(|x| x as f32)
                .collect();
            if v.is_empty() {
                return Err(io_msg("an embeddings entry is an empty vector"));
            }
            // Stored normalized so cosine is a dot product, exactly as
            // HashEmbedder's output is.
            normalize(&mut v);
            out.push(v);
        }
        Ok(out)
    }
}

fn io_err<E: std::fmt::Display>(e: E) -> StoreError {
    io_msg(&e.to_string())
}

fn io_msg(msg: &str) -> StoreError {
    StoreError::Io(std::io::Error::other(msg.to_string()))
}

/// Pick the embedder a production path should use, and say which one it
/// got. Never falls back quietly: #163 exists because `hybrid=true` kept
/// returning plausibly-ordered hits with no model behind them, and
/// nothing in the logs or the Ledger said so.
pub fn resolve() -> Chosen {
    resolve_with(InferencedEmbedder::default_socket())
}

/// What [`resolve`] picked, and enough to say so out loud.
///
/// `kind` alone was not enough: "inferenced" covers both the pinned
/// embedding model and the daemon's default chat model, and those give
/// materially different retrieval. The Ledger and the CLI report both.
pub struct Chosen {
    pub embedder: Box<dyn Embedder>,
    pub kind: &'static str,
    pub model: Option<String>,
}

/// The testable half of [`resolve`]: the socket comes in as an argument
/// so a test can point it at a stub without mutating the process
/// environment, which is both unsafe and shared between parallel tests.
pub fn resolve_with(socket: Option<std::path::PathBuf>) -> Chosen {
    // Connect, don't stat. A socket file outlives the daemon that made
    // it, so existence would happily select a dead endpoint and turn a
    // startup-time report into a failure at the first embed.
    if let Some(path) = &socket
        && std::os::unix::net::UnixStream::connect(path).is_ok()
    {
        // Name the embedding model when the daemon has it; stay silent
        // and take the daemon's default when it does not. Both are the
        // model-backed path — the difference is quality, not kind, and
        // the Ledger says which via `embedder_model`.
        let model = InferencedEmbedder::preferred_model(path);
        if model.is_none() {
            tracing::info!(
                want = EMBEDDING_MODEL,
                "the embedding model is not loaded; embedding with the daemon's default \
                 (a chat model, mean-pooled). `lisa models get {}` improves retrieval quality.",
                EMBEDDING_MODEL
            );
        }
        return Chosen {
            embedder: Box::new(InferencedEmbedder::new(path.clone(), model.clone())),
            kind: "inferenced",
            model,
        };
    }
    tracing::warn!(
        socket = ?socket,
        "no model-backed embedder: falling back to HashEmbedder, which has no semantic model \
         behind it. Hybrid search will still return results, and they will still look ranked — \
         they are lexical only (#163)."
    );
    Chosen {
        embedder: Box::new(HashEmbedder::default()),
        kind: "hash",
        model: None,
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

    /// Read one whole HTTP request: headers, then `Content-Length`
    /// bytes of body.
    ///
    /// A stub that reads ONCE and replies is a race, not a server. The
    /// client writes its headers and its body as two separate
    /// `write_all` calls (see `post_json`), so a single `read` may
    /// return the headers alone — at which point a stub that answers
    /// and closes leaves the client's second write aimed at a dead
    /// peer, and `embed()` fails with `BrokenPipe`. That is not a
    /// hypothetical: it turned `a_reachable_socket_is_used_...` red in
    /// CI while passing on every developer machine, because it only
    /// bites when the read wins the race against the second write.
    ///
    /// Returns false if the peer hung up before completing a request —
    /// which is what the probe connection in `resolve_with` does, by
    /// design.
    fn drain_request(sock: &mut std::os::unix::net::UnixStream) -> bool {
        use std::io::Read;
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        // Headers, byte at a time. Slow and completely correct — this
        // is a test stub, and the alternative is re-implementing
        // buffered parsing with a leftover-bytes bug in it.
        while !buf.ends_with(b"\r\n\r\n") {
            match sock.read(&mut byte) {
                Ok(0) | Err(_) => return false,
                Ok(_) => buf.push(byte[0]),
            }
        }
        let len = String::from_utf8_lossy(&buf)
            .lines()
            .find_map(|l| {
                l.split_once(':').and_then(|(k, v)| {
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        let mut body = vec![0u8; len];
        sock.read_exact(&mut body).is_ok()
    }

    /// A stand-in for lisa-inferenced's `/v1/embeddings` on a unix
    /// socket. Answers `n` requests then stops; every vector it returns
    /// is a distinctive constant so a test can tell its output apart
    /// from anything HashEmbedder would produce.
    fn stub_embeddings_server(
        path: std::path::PathBuf,
        requests: usize,
    ) -> std::thread::JoinHandle<()> {
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            use std::io::Write;
            // Counts ANSWERED requests, not accepted connections.
            // `resolve_with` probes by connecting and hanging up
            // immediately; charging that probe against the budget is
            // how the caller ends up counting connections in a comment
            // instead of stating what it needs answered.
            let mut answered = 0;
            while answered < requests {
                let Ok((mut sock, _)) = listener.accept() else {
                    return;
                };
                if !drain_request(&mut sock) {
                    continue;
                }
                answered += 1;
                let body = serde_json::json!({
                    "data": [{ "embedding": [3.0, 4.0] }]
                })
                .to_string();
                let _ = sock.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                         Connection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        })
    }

    /// A stand-in for GET /v1/models.
    fn stub_models_server(
        path: std::path::PathBuf,
        ids: Vec<String>,
    ) -> std::thread::JoinHandle<()> {
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            use std::io::Write;
            while let Ok((mut sock, _)) = listener.accept() {
                if !drain_request(&mut sock) {
                    continue;
                }
                let data: Vec<_> = ids
                    .iter()
                    .map(|id| serde_json::json!({ "id": id, "object": "model" }))
                    .collect();
                let body = serde_json::json!({ "object": "list", "data": data }).to_string();
                let _ = sock.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                         Connection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
                // Exactly one ANSWERED request, so `join()` returns.
                return;
            }
        })
    }

    /// #163: a reachable model-backed embedder must actually be used.
    /// The stub returns [3,4], which normalizes to [0.6,0.8] — a
    /// two-element vector HashEmbedder (64 dims of token counts) can
    /// never produce, so this asserts the model's numbers arrived rather
    /// than merely that something was selected.
    #[test]
    fn a_reachable_socket_is_used_instead_of_the_hash_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("inferenced.sock");
        // TWO answered requests: /v1/models, then the embeddings POST.
        // (resolve_with also probes by connecting and hanging up; the
        // stub does not count a request it never answered.) The stub
        // replies to everything with an embeddings body, so the models
        // query finds no "id" and correctly yields no model — which is
        // what this test wants, since it is asserting the embedder
        // choice, not the model choice.
        let server = stub_embeddings_server(sock.clone(), 2);

        let chosen = resolve_with(Some(sock.clone()));
        assert_eq!(
            chosen.kind, "inferenced",
            "a live socket must win over the fallback"
        );
        let out = chosen.embedder.embed(&["anything".to_string()]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].len(),
            2,
            "vector came from somewhere other than the socket"
        );
        assert!((out[0][0] - 0.6).abs() < 1e-6 && (out[0][1] - 0.8).abs() < 1e-6);
        let _ = server.join();

        // POSITIVE CONTROL. The assertions above pass for a test that
        // simply never reaches the fallback path; this is the same call
        // with the socket taken away, and it must come back "hash".
        // Without it, deleting InferencedEmbedder entirely would leave
        // the suite green in the only way that matters.
        let gone = dir.path().join("not-a-socket.sock");
        let fallback = resolve_with(Some(gone));
        assert_eq!(
            fallback.kind, "hash",
            "an unreachable socket must fall back, loudly"
        );
        assert!(fallback.model.is_none(), "the fallback names no model");
        let out = fallback.embedder.embed(&["anything".to_string()]).unwrap();
        assert_eq!(out[0].len(), 64, "the fallback is HashEmbedder's 64 dims");
    }

    /// #163's second half: naming the model. The stub answers
    /// /v1/models with the catalog's embedding model, so the embedder
    /// must ASK for it — and when the daemon lists only a chat model it
    /// must ask for nothing rather than for a model that is not there,
    /// which would turn "embeds adequately" into "cannot embed".
    #[test]
    fn the_embedding_model_is_named_only_when_the_daemon_has_it() {
        let dir = tempfile::tempdir().unwrap();

        let with = dir.path().join("with.sock");
        let s1 = stub_models_server(with.clone(), vec![EMBEDDING_MODEL.to_string()]);
        assert_eq!(
            InferencedEmbedder::preferred_model(&with).as_deref(),
            Some(EMBEDDING_MODEL)
        );
        let _ = s1.join();

        // POSITIVE CONTROL: the same call against a daemon that has only
        // a chat model must come back None. Without this, a
        // preferred_model() that returned Some unconditionally would
        // also pass the assertion above.
        let without = dir.path().join("without.sock");
        let s2 = stub_models_server(without.clone(), vec!["qwen3-1.7b-instruct-q8".into()]);
        assert_eq!(InferencedEmbedder::preferred_model(&without), None);
        let _ = s2.join();
    }

    /// A truncated reply must be an error, not a silently short set of
    /// vectors: chunks and vectors are matched by position, so one
    /// missing vector gives every document after it someone else's
    /// meaning.
    #[test]
    fn a_short_reply_is_refused_rather_than_misaligned() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("short.sock");
        let server = stub_embeddings_server(sock.clone(), 1);
        let embedder = InferencedEmbedder::new(sock, None);
        let err = embedder
            .embed(&["one".to_string(), "two".to_string()])
            .unwrap_err();
        assert!(
            err.to_string().contains("1 vectors for 2 inputs"),
            "expected a count mismatch, got: {err}"
        );
        let _ = server.join();
    }

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
