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

/// Wraps an embedder so a *cold* one is waited for instead of being
/// fatal — with a budget, so the wait cannot outlive the caller.
///
/// `After=lisa-inferenced-dbus.service` orders start, not readiness:
/// llama-server takes tens of seconds to load nomic-embed, so the first
/// POST of a boot can meet a socket that accepts and then hangs up
/// (`Broken pipe`) or is not there yet (`No such file`). Retrying that
/// a few times costs seconds; not retrying it costs the whole pass.
///
/// Bounded on purpose (#192): `attempts` total tries with delays of
/// `base_delay * 2^n`, so the worst case is arithmetic a reader can do
/// — 5 attempts at 2s is 2+4+8+16 = 30 seconds of waiting, inside a
/// 120-second oneshot. Only errors a wait can fix are retried; an HTTP
/// 400 for a model that is not loaded answers just as fast the fourth
/// time.
pub struct RetryingEmbedder<'a> {
    inner: &'a dyn Embedder,
    attempts: u32,
    base_delay: std::time::Duration,
}

impl<'a> RetryingEmbedder<'a> {
    pub fn new(inner: &'a dyn Embedder, attempts: u32, base_delay: std::time::Duration) -> Self {
        Self {
            inner,
            attempts: attempts.max(1),
            base_delay,
        }
    }

    /// Worst-case time this wrapper can spend asleep. The caller's
    /// timeout budget has to be bigger than this number, and a number
    /// is the only way to check that.
    pub fn max_backoff(&self) -> std::time::Duration {
        (0..self.attempts.saturating_sub(1))
            .map(|n| self.base_delay * 2u32.saturating_pow(n))
            .sum()
    }
}

/// Whether waiting could plausibly change the answer. Transport-level
/// failures against a daemon that is still starting: yes. Anything the
/// server actually answered — a refusal, a malformed body, a count
/// mismatch — arrives as [`std::io::ErrorKind::Other`] via `io_msg`
/// and is not retried.
fn is_transient(err: &StoreError) -> bool {
    let StoreError::Io(e) = err else { return false };
    matches!(
        e.kind(),
        std::io::ErrorKind::NotFound
            | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock
    )
}

impl Embedder for RetryingEmbedder<'_> {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, StoreError> {
        let mut delay = self.base_delay;
        for attempt in 1..=self.attempts {
            match self.inner.embed(texts) {
                Ok(v) => return Ok(v),
                Err(e) if attempt < self.attempts && is_transient(&e) => {
                    tracing::info!(
                        attempt,
                        of = self.attempts,
                        wait_ms = delay.as_millis() as u64,
                        error = %e,
                        "the embedder is not answering yet; waiting before the next try"
                    );
                    std::thread::sleep(delay);
                    delay *= 2;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("the loop returns on the last attempt")
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

/// One query, one vector — both hybrid paths embed the query exactly
/// once, up front, whether or not the lexical prefilter found anything
/// (#176's fallthrough needs the vector precisely when it found
/// nothing).
fn embed_query(embedder: &dyn Embedder, query: &str) -> Result<Vec<f32>, StoreError> {
    let mut v = embedder.embed(std::slice::from_ref(&query.to_string()))?;
    Ok(v.remove(0))
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
        self.embed_pending_batched(embedder, 64)
    }

    /// Embed the pending chunks of ONE provenance, leaving every other
    /// queue exactly where it was.
    ///
    /// #192: `lisa-knowledge-sync` indexed its 28 knowledge-pack chunks
    /// in five seconds, then called the unscoped [`Self::embed_pending`]
    /// and inherited the ~90,000 `mail` chunks the #170 backfill had
    /// left pending. systemd SIGTERM'd it at `TimeoutStartSec=120` —
    /// on every boot, until the backfill happened to finish elsewhere.
    /// A caller that owns one provenance now says so, and its runtime
    /// is bounded by its own corpus instead of by whatever else the
    /// store is carrying.
    ///
    /// The unscoped variant stays: an explicit, long-running command
    /// like `lisa context index --embed` or the mail backfill is
    /// *supposed* to drain the whole queue.
    pub fn embed_pending_provenance(
        &self,
        embedder: &dyn Embedder,
        provenance: &str,
    ) -> Result<usize, StoreError> {
        // #104's rule again: an unknown tag matches no rows, and
        // "embedded 0 chunks" is indistinguishable from "already done".
        if !crate::acl::is_known_provenance(provenance) {
            return Err(StoreError::UnknownProvenance(provenance.to_string()));
        }
        self.embed_pending_inner(embedder, 64, Some(provenance))
    }

    /// The batch size exists because the first real mailbox proved it
    /// must: 31,845 pending chunks went out as ONE /v1/embeddings POST
    /// — megabytes of JSON — and llama-server hung up (broken pipe,
    /// device, 2026-08-04). Stub-served tests can never see that.
    /// Vectors land per batch inside a transaction, so an interrupted
    /// run keeps everything embedded so far and resumes where it
    /// stopped; the connection lock is released between batches so a
    /// minutes-long backfill does not starve readers.
    pub fn embed_pending_batched(
        &self,
        embedder: &dyn Embedder,
        batch: usize,
    ) -> Result<usize, StoreError> {
        self.embed_pending_inner(embedder, batch, None)
    }

    /// The shared loop. `provenance = None` is the whole store; `Some`
    /// restricts it to one tag's documents.
    fn embed_pending_inner(
        &self,
        embedder: &dyn Embedder,
        batch: usize,
        provenance: Option<&str>,
    ) -> Result<usize, StoreError> {
        let batch = batch.max(1);
        let mut done = 0usize;
        loop {
            let pending: Vec<(i64, i64, String)> = {
                let conn = self.conn.lock().expect("context lock");
                match provenance {
                    // Left exactly as it was: no join, so a chunk whose
                    // document row is missing still gets a vector here.
                    None => {
                        let mut stmt = conn.prepare(
                            "SELECT c.doc_id, c.seq, c.content
                             FROM chunks c
                             LEFT JOIN chunk_vectors v ON v.doc_id = c.doc_id AND v.seq = c.seq
                             WHERE v.doc_id IS NULL
                             LIMIT ?1",
                        )?;
                        stmt.query_map([batch as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                            .collect::<Result<_, _>>()?
                    }
                    Some(p) => {
                        let mut stmt = conn.prepare(
                            "SELECT c.doc_id, c.seq, c.content
                             FROM chunks c
                             JOIN documents d ON d.id = c.doc_id
                             LEFT JOIN chunk_vectors v ON v.doc_id = c.doc_id AND v.seq = c.seq
                             WHERE v.doc_id IS NULL AND d.provenance = ?2
                             LIMIT ?1",
                        )?;
                        stmt.query_map(rusqlite::params![batch as i64, p], |r| {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                        })?
                        .collect::<Result<_, _>>()?
                    }
                }
            };
            if pending.is_empty() {
                return Ok(done);
            }
            let texts: Vec<String> = pending.iter().map(|(_, _, c)| c.clone()).collect();
            let vectors = embedder.embed(&texts)?;
            {
                let conn = self.conn.lock().expect("context lock");
                let tx = conn.unchecked_transaction()?;
                for ((doc_id, seq, _), vec) in pending.iter().zip(vectors.iter()) {
                    tx.execute(
                        "INSERT OR REPLACE INTO chunk_vectors (doc_id, seq, dim, vec)
                         VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![doc_id, seq, vec.len() as i64, vec_to_blob(vec)],
                    )?;
                }
                tx.commit()?;
            }
            done += pending.len();
            if done.is_multiple_of(6400) {
                tracing::info!(embedded = done, "embedding backfill progress");
            }
        }
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
        let mut candidates = self.search(query, limit.max(20) * 3)?;
        let qvec = embed_query(embedder, query)?;
        // Fallthrough (#176): when lexical search cannot fill the ask,
        // the vectors get their chance directly. Without this, a query
        // sharing no words with a document can NEVER surface it — the
        // prefilter returns nothing and there is nothing to rerank,
        // which on the device turned "bills from the internet provider"
        // into zero results against an indexed, embedded invoice.
        // Brute force is fine at this scale: the vectors live as SQLite
        // blobs and a full scan of a few thousand is milliseconds
        // (sqlite-vec remains the later optimization, same as the
        // schema comment says).
        if candidates.len() < limit {
            let extra = self.vector_candidates(&qvec, &candidates, None, limit)?;
            candidates.extend(extra);
        }
        self.rerank(&qvec, candidates, limit)
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
        let mut candidates = self.search_scoped(query, scopes, limit.max(20) * 3)?;
        let qvec = embed_query(embedder, query)?;
        // Same fallthrough as search_hybrid, with the ACL applied to
        // the vector scan too: a disallowed chunk must not become a
        // candidate through the vector door when the lexical door
        // already refused it (the orthogonality this function's doc
        // comment exists to defend).
        if candidates.len() < limit {
            let extra = self.vector_candidates(&qvec, &candidates, Some(scopes), limit)?;
            candidates.extend(extra);
        }
        self.rerank(&qvec, candidates, limit)
    }

    /// Top-`k` chunks by cosine against `qvec`, excluding sources
    /// already among `have`, optionally restricted to `scopes`
    /// (provenance values). Score is 0.0 — bm25's "no lexical match at
    /// all" — so in the blend these compete on similarity alone and a
    /// lexical hit of equal similarity always outranks them.
    fn vector_candidates(
        &self,
        qvec: &[f32],
        have: &[Hit],
        scopes: Option<&[&str]>,
        k: usize,
    ) -> Result<Vec<Hit>, StoreError> {
        let seen: std::collections::HashSet<&str> =
            have.iter().map(|h| h.source.as_str()).collect();
        let conn = self.conn.lock().expect("context lock");
        let mut stmt = conn.prepare(
            "SELECT d.source, d.provenance, c.content, v.vec
             FROM chunk_vectors v
             JOIN documents d ON d.id = v.doc_id
             JOIN chunks c ON c.doc_id = v.doc_id AND c.seq = v.seq",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Vec<u8>>(3)?,
            ))
        })?;

        let mut best: std::collections::HashMap<String, (f64, Hit)> =
            std::collections::HashMap::new();
        for row in rows {
            let (source, provenance, content, blob) = row?;
            if seen.contains(source.as_str()) {
                continue;
            }
            if let Some(allowed) = scopes
                && !allowed.contains(&provenance.as_str())
            {
                continue;
            }
            let cos = cosine(qvec, &blob_to_vec(&blob)) as f64;
            let snippet: String = content.chars().take(160).collect();
            let hit = Hit {
                source: source.clone(),
                provenance,
                snippet,
                score: 0.0,
            };
            match best.get(&source) {
                Some((prev, _)) if *prev >= cos => {}
                _ => {
                    best.insert(source, (cos, hit));
                }
            }
        }
        let mut ranked: Vec<(f64, Hit)> = best.into_values().collect();
        ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(ranked.into_iter().take(k).map(|(_, h)| h).collect())
    }

    fn rerank(
        &self,
        qvec: &[f32],
        candidates: Vec<Hit>,
        limit: usize,
    ) -> Result<Vec<Hit>, StoreError> {
        if candidates.is_empty() {
            return Ok(candidates);
        }
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

    /// #the-31k-lesson: pending chunks embed in BATCHES — one request
    /// per batch, resumable, never one giant POST. The counting stub
    /// asserts the request count.
    #[test]
    fn embedding_backfill_batches_and_resumes() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(
                dir.path().join(format!("d{i}.md")),
                format!("document number {i}"),
            )
            .unwrap();
        }
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        store.index_dir(dir.path()).unwrap();

        struct Counting(std::sync::atomic::AtomicUsize);
        impl Embedder for Counting {
            fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, StoreError> {
                assert!(
                    texts.len() <= 2,
                    "batch leaked: {} texts in one request",
                    texts.len()
                );
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
            }
        }
        let counter = Counting(std::sync::atomic::AtomicUsize::new(0));
        let n = store.embed_pending_batched(&counter, 2).unwrap();
        assert_eq!(n, 5, "all pending embedded");
        assert_eq!(
            counter.0.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "5 chunks at batch=2 is exactly 3 requests"
        );
        // Resumability: nothing pending → zero requests.
        assert_eq!(store.embed_pending_batched(&counter, 2).unwrap(), 0);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    /// How many chunks of `provenance` still have no vector.
    fn pending_of(store: &ContextStore, provenance: &str) -> usize {
        let conn = store.conn.lock().unwrap();
        conn.query_row(
            "SELECT count(*)
             FROM chunks c
             JOIN documents d ON d.id = c.doc_id
             LEFT JOIN chunk_vectors v ON v.doc_id = c.doc_id AND v.seq = c.seq
             WHERE v.doc_id IS NULL AND d.provenance = ?1",
            [provenance],
            |r| r.get::<_, i64>(0),
        )
        .unwrap() as usize
    }

    /// #192, from the reference device: `lisa-knowledge-sync` indexed
    /// its 28 chunks in five seconds and was then SIGTERM'd at
    /// `TimeoutStartSec=120`, because the embed step it ran afterwards
    /// was UNSCOPED — it inherited ~90,000 pending `mail` chunks from
    /// the #170 backfill. A unit that owns the knowledge pack must
    /// embed the knowledge pack, and leave every other queue exactly
    /// where it found it.
    #[test]
    fn a_scoped_embed_leaves_another_provenance_pending() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path().join("pack");
        let mail = dir.path().join("mail");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::create_dir_all(&mail).unwrap();
        std::fs::write(
            sys.join("about.md"),
            "Lisa OS keeps local models as a system service.",
        )
        .unwrap();
        for i in 0..4 {
            std::fs::write(
                mail.join(format!("m{i}.md")),
                format!("mail message {i} about the quarterly invoice"),
            )
            .unwrap();
        }
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        let pack = store.index_dir_as(&sys, "system").unwrap();
        let inbox = store.index_dir_as(&mail, "mail").unwrap();
        assert!(pack.chunks > 0 && inbox.chunks > 0, "nothing to embed");

        /// Records what actually crossed the wire, so the assertion is
        /// about the texts sent to the model and not only a count.
        #[derive(Default)]
        struct Recording(std::sync::Mutex<Vec<String>>);
        impl Embedder for Recording {
            fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, StoreError> {
                self.0.lock().unwrap().extend(texts.iter().cloned());
                Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
            }
        }

        let rec = Recording::default();
        let n = store.embed_pending_provenance(&rec, "system").unwrap();
        assert_eq!(
            n, pack.chunks,
            "a `system`-scoped embed reported {n} chunks for a pack of {}",
            pack.chunks
        );
        let seen = rec.0.lock().unwrap().clone();
        assert!(
            !seen.iter().any(|t| t.contains("mail message")),
            "the scoped embed drained another provenance's queue: {seen:?}"
        );
        assert_eq!(pending_of(&store, "system"), 0, "the pack stayed pending");
        assert_eq!(
            pending_of(&store, "mail"),
            inbox.chunks,
            "the mail queue moved; the knowledge sync is not its owner"
        );
    }

    /// The other half of #192: an unknown tag is refused rather than
    /// quietly matching nothing, which would look exactly like "the
    /// pack was already embedded".
    #[test]
    fn a_scoped_embed_refuses_an_unknown_provenance() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "anything at all").unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        store.index_dir(dir.path()).unwrap();
        let err = store
            .embed_pending_provenance(&HashEmbedder::default(), "System")
            .unwrap_err();
        assert!(
            matches!(err, StoreError::UnknownProvenance(ref p) if p == "System"),
            "expected a refusal, got: {err}"
        );
        assert_eq!(
            pending_of(&store, "file"),
            1,
            "a refused call must not have embedded anything"
        );
    }

    /// An embedder that fails `fails` times with `kind`, then answers.
    struct Flaky {
        fails: usize,
        kind: std::io::ErrorKind,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl Flaky {
        fn new(fails: usize, kind: std::io::ErrorKind) -> Self {
            Self {
                fails,
                kind,
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }
    impl Embedder for Flaky {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, StoreError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.fails {
                return Err(StoreError::Io(std::io::Error::from(self.kind)));
            }
            Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
        }
    }

    /// #192's original suspicion, which is real even though it is not
    /// what killed the unit: `After=` orders start, not readiness, so
    /// the first POST after a cold boot can meet a llama-server still
    /// loading nomic-embed and get a broken pipe. Bounded retries turn
    /// that into a wait; the budget is what keeps it from turning into
    /// a hang inside a 120-second oneshot.
    #[test]
    fn a_cold_embedder_is_retried_within_a_bounded_budget() {
        // The number the unit file's timeout is justified against: the
        // knowledge sync's 5 attempts at 2s can sleep 2+4+8+16 = 30
        // seconds and no more. A comment claiming that is a claim; this
        // is the claim being checked.
        assert_eq!(
            RetryingEmbedder::new(
                &HashEmbedder::default(),
                5,
                std::time::Duration::from_secs(2)
            )
            .max_backoff(),
            std::time::Duration::from_secs(30),
            "the retry budget is not what the unit file's TimeoutStartSec assumes"
        );

        let flaky = Flaky::new(2, std::io::ErrorKind::BrokenPipe);
        let retrying = RetryingEmbedder::new(&flaky, 4, std::time::Duration::from_millis(1));
        let out = retrying.embed(&["one".to_string()]).unwrap();
        assert_eq!(out.len(), 1, "the retry must return the successful answer");
        assert_eq!(flaky.calls(), 3, "expected two failures then one success");

        // POSITIVE CONTROL: with more failures than the budget allows,
        // the error must come back — otherwise this test would pass
        // against an implementation that retries forever, which is the
        // failure mode being avoided.
        let stubborn = Flaky::new(99, std::io::ErrorKind::ConnectionRefused);
        let retrying = RetryingEmbedder::new(&stubborn, 3, std::time::Duration::from_millis(1));
        let err = retrying.embed(&["one".to_string()]).unwrap_err();
        assert_eq!(stubborn.calls(), 3, "the attempt budget was not honoured");
        assert!(
            err.to_string().to_lowercase().contains("refused"),
            "the last error must survive the retries: {err}"
        );
    }

    /// Retry only what a wait can fix. "No such model" answers just as
    /// fast the fourth time, and retrying it would spend the whole
    /// budget on a certainty.
    #[test]
    fn a_permanent_error_is_not_retried() {
        struct Refusing(std::sync::atomic::AtomicUsize);
        impl Embedder for Refusing {
            fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, StoreError> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(io_msg("embeddings returned HTTP 400: no such model"))
            }
        }
        // Seconds, not milliseconds: an implementation that retries
        // this must make the suite visibly slower, so the regression is
        // felt and not just counted. (Mutating `is_transient` to accept
        // everything took this test from 0.00s to 210s before it failed
        // on the count — hence 5s rather than 30s here.)
        let refusing = Refusing(std::sync::atomic::AtomicUsize::new(0));
        let retrying = RetryingEmbedder::new(&refusing, 4, std::time::Duration::from_secs(5));
        let err = retrying.embed(&["one".to_string()]).unwrap_err();
        assert_eq!(
            refusing.0.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a permanent refusal was retried"
        );
        assert!(err.to_string().contains("no such model"), "got: {err}");
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

    /// An embedder whose notion of similarity is a lookup table: any
    /// text containing a key maps to that key's vector. Deterministic
    /// semantic neighborhoods for tests, with none of HashEmbedder's
    /// token-overlap accidents.
    struct TableEmbedder(Vec<(&'static str, Vec<f32>)>);
    impl Embedder for TableEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, StoreError> {
            Ok(texts
                .iter()
                .map(|t| {
                    self.0
                        .iter()
                        .find(|(k, _)| t.contains(k))
                        .map(|(_, v)| v.clone())
                        .unwrap_or_else(|| vec![0.0, 0.0, 1.0])
                })
                .collect())
        }
    }

    /// #176, reproduced from the reference device: an indexed, embedded
    /// document containing "quarterly invoices from Negenet" returned
    /// ZERO hits for "bills from the internet provider" — no lexical
    /// overlap, so the BM25 prefilter produced no candidates and the
    /// vectors never got a chance. The fallthrough gives them one.
    #[test]
    fn a_query_sharing_no_words_still_finds_its_document() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("invoice.md"),
            "Fatura: quarterly invoices from Negenet arrive by email.",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("soup.md"),
            "A recipe for lentil soup with cumin.",
        )
        .unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        store.index_dir(dir.path()).unwrap();

        let embedder = TableEmbedder(vec![
            ("invoices", vec![1.0, 0.0, 0.0]),
            ("bills", vec![0.9, 0.1, 0.0]), // near the invoice doc
            ("lentil", vec![0.0, 1.0, 0.0]),
        ]);
        store.embed_pending(&embedder).unwrap();

        // POSITIVE CONTROL first: prove this query really has no
        // lexical path, or the assertion below proves nothing about
        // the fallthrough.
        assert!(
            store
                .search("bills from the internet provider", 5)
                .unwrap()
                .is_empty(),
            "the control failed: lexical search found something, so this \
             test would pass without the fallthrough"
        );

        let hits = store
            .search_hybrid("bills from the internet provider", &embedder, 3)
            .unwrap();
        assert!(
            hits.first()
                .is_some_and(|h| h.source.ends_with("invoice.md")),
            "the vector fallthrough must surface the invoice doc: {hits:?}"
        );
    }

    /// The ACL is not bypassable through the vector door: the same
    /// zero-overlap query with a scope that excludes the document's
    /// provenance must stay empty.
    #[test]
    fn the_vector_fallthrough_respects_scopes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("invoice.md"),
            "Fatura: quarterly invoices from Negenet arrive by email.",
        )
        .unwrap();
        let store = ContextStore::open(dir.path().join("ctx.db")).unwrap();
        store.index_dir(dir.path()).unwrap(); // provenance: file

        let embedder = TableEmbedder(vec![
            ("invoices", vec![1.0, 0.0, 0.0]),
            ("bills", vec![0.9, 0.1, 0.0]),
        ]);
        store.embed_pending(&embedder).unwrap();

        // Allowed scope → found through the fallthrough.
        let allowed = store
            .search_hybrid_scoped("bills from the internet provider", &["file"], &embedder, 3)
            .unwrap();
        assert!(
            !allowed.is_empty(),
            "positive control: scope 'file' finds it"
        );

        // Disallowed scope → the vector door is shut too.
        let denied = store
            .search_hybrid_scoped("bills from the internet provider", &["mail"], &embedder, 3)
            .unwrap();
        assert!(
            denied.is_empty(),
            "a chunk outside the scope surfaced through the vector scan: {denied:?}"
        );
    }
}
