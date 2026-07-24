//! Remote-provider routing (`docs/PLAN.md` §5.11). Model names of the
//! form `remote:<provider>:<model>` are forwarded to the `lisa-remoted`
//! egress broker over its unix socket — inferenced itself stays
//! network-free (rule 5). The broker enforces per-scope consent and the
//! ledger `remote.` marking; inferenced only proxies.
//!
//! Transport: a minimal HTTP/1.1 POST over the unix socket with
//! `Connection: close`, no extra HTTP-client dependency and no TCP.
//! Streaming is real (ADR-0010 update): the request carries
//! `stream:true`, the broker answers `text/event-stream` (chunked), and
//! the SSE `data:` frames — OpenAI `chat.completion.chunk` shape for
//! every provider dialect — are decoded incrementally and yielded as
//! true token deltas. Mid-stream `{"error":...}` frames and early EOF
//! surface as engine errors; an idle read timeout keeps a stalled broker
//! or provider from hanging a session.

use crate::engine::{Engine, EngineError, GenerateRequest, TokenStream};
use crate::pool::EngineProvider;
use futures::future::BoxFuture;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;

pub const REMOTE_PREFIX: &str = "remote:";

/// A silent broker/provider must not hang a chat: give up when no bytes
/// arrive for this long (the broker applies its own upstream idle
/// timeout and reports stalls as error frames well before this fires).
const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(150);

/// Default broker socket (matches lisa-remoted's StateDirectory).
pub fn default_socket() -> PathBuf {
    std::env::var_os("LISA_REMOTED_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/lisa/remoted/remoted.sock"))
}

/// Split `remote:<provider>:<model>` → (provider, model). Model may
/// contain further colons/slashes (HF ids like `org/model:policy`).
fn parse(model: &str) -> Option<(String, String)> {
    let rest = model.strip_prefix(REMOTE_PREFIX)?;
    let (provider, inner) = rest.split_once(':')?;
    if provider.is_empty() || inner.is_empty() {
        return None;
    }
    Some((provider.to_string(), inner.to_string()))
}

fn unavailable(msg: impl Into<String>) -> EngineError {
    EngineError::Unavailable(msg.into())
}

/// One read with the idle timeout applied; `Ok(0)` is EOF.
async fn read_some<S: AsyncRead + Unpin>(io: &mut S, buf: &mut [u8]) -> Result<usize, EngineError> {
    match tokio::time::timeout(IDLE_READ_TIMEOUT, io.read(buf)).await {
        Err(_) => Err(unavailable(format!(
            "remoted stream stalled (no bytes for {}s)",
            IDLE_READ_TIMEOUT.as_secs()
        ))),
        Ok(Err(e)) => Err(unavailable(format!("remoted read: {e}"))),
        Ok(Ok(n)) => Ok(n),
    }
}

/// Incremental HTTP/1.1 chunked-transfer decoder — the broker's
/// streamed responses have no Content-Length, so hyper sends them
/// chunked. Feed raw bytes, collect decoded payload; returns `true`
/// once the terminal 0-size chunk has been seen (trailers ignored).
#[derive(Default)]
struct ChunkedDecoder {
    state: ChunkState,
    remaining: usize,
    line: Vec<u8>,
}

#[derive(Default, PartialEq)]
enum ChunkState {
    #[default]
    Size,
    Data,
    DataEnd,
    Done,
}

impl ChunkedDecoder {
    fn feed(&mut self, mut input: &[u8], out: &mut Vec<u8>) -> Result<bool, EngineError> {
        while !input.is_empty() {
            match self.state {
                ChunkState::Size => {
                    let Some(pos) = input.iter().position(|&b| b == b'\n') else {
                        self.line.extend_from_slice(input);
                        break;
                    };
                    self.line.extend_from_slice(&input[..pos]);
                    input = &input[pos + 1..];
                    let line = String::from_utf8_lossy(&self.line).trim().to_string();
                    self.line.clear();
                    let hex = line.split(';').next().unwrap_or("").trim();
                    let size = usize::from_str_radix(hex, 16).map_err(|_| {
                        unavailable(format!("remoted: bad chunk size line {line:?}"))
                    })?;
                    if size == 0 {
                        self.state = ChunkState::Done;
                        return Ok(true);
                    }
                    self.remaining = size;
                    self.state = ChunkState::Data;
                }
                ChunkState::Data => {
                    let take = self.remaining.min(input.len());
                    out.extend_from_slice(&input[..take]);
                    self.remaining -= take;
                    input = &input[take..];
                    if self.remaining == 0 {
                        self.state = ChunkState::DataEnd;
                    }
                }
                ChunkState::DataEnd => {
                    // Skip the CRLF that terminates the chunk data.
                    let Some(pos) = input.iter().position(|&b| b == b'\n') else {
                        break;
                    };
                    input = &input[pos + 1..];
                    self.state = ChunkState::Size;
                }
                ChunkState::Done => return Ok(true),
            }
        }
        Ok(self.state == ChunkState::Done)
    }
}

/// Incremental SSE frame parser: feed bytes, get complete `data:`
/// payloads. Same contract as the broker's parser: events split across
/// reads, `\n`/`\r\n` endings, multi-line data, comment/`event:` lines
/// skipped; boundaries are ASCII so UTF-8 never splits.
#[derive(Default)]
struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some((end, sep)) = event_boundary(&self.buf) {
            let event: Vec<u8> = self.buf.drain(..end + sep).collect();
            let text = String::from_utf8_lossy(&event[..end]);
            let data: Vec<&str> = text
                .lines()
                .map(|l| l.strip_suffix('\r').unwrap_or(l))
                .filter_map(|l| l.strip_prefix("data:"))
                .map(|l| l.strip_prefix(' ').unwrap_or(l))
                .collect();
            if !data.is_empty() {
                out.push(data.join("\n"));
            }
        }
        out
    }
}

fn event_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    for i in 0..buf.len() {
        if buf[i] == b'\n' {
            if buf.get(i + 1) == Some(&b'\n') {
                return Some((i, 2));
            }
            if buf.get(i + 1) == Some(&b'\r') && buf.get(i + 2) == Some(&b'\n') {
                return Some((i, 3));
            }
        }
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case(name)
            .then(|| v.trim().to_ascii_lowercase())
    })
}

/// The parsed response head plus whatever body bytes arrived with it.
struct ResponseHead {
    status_ok: bool,
    chunked: bool,
    sse: bool,
    rest: Vec<u8>,
}

async fn read_head<S: AsyncRead + Unpin>(io: &mut S) -> Result<ResponseHead, EngineError> {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos;
        }
        let n = read_some(io, &mut tmp).await?;
        if n == 0 {
            return Err(unavailable("remoted: connection closed before response"));
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let rest = buf.split_off(head_end + 4);
    Ok(ResponseHead {
        status_ok: head.lines().next().is_some_and(|l| l.contains(" 200 ")),
        chunked: header_value(&head, "transfer-encoding").is_some_and(|v| v.contains("chunked")),
        sse: header_value(&head, "content-type").is_some_and(|v| v.contains("text/event-stream")),
        rest,
    })
}

/// Collect a whole (non-SSE) body: chunked-decoded if flagged, else to
/// EOF — the request always sends `Connection: close`.
async fn read_body<S: AsyncRead + Unpin>(
    io: &mut S,
    head: &ResponseHead,
) -> Result<Vec<u8>, EngineError> {
    let mut decoder = head.chunked.then(ChunkedDecoder::default);
    let mut body = Vec::new();
    let mut input = head.rest.clone();
    let mut tmp = [0u8; 4096];
    loop {
        match &mut decoder {
            Some(d) => {
                if d.feed(&input, &mut body)? {
                    return Ok(body);
                }
            }
            None => body.extend_from_slice(&input),
        }
        let n = read_some(io, &mut tmp).await?;
        if n == 0 {
            return Ok(body);
        }
        input = tmp[..n].to_vec();
    }
}

fn error_from_body(body: &[u8]) -> EngineError {
    let text = String::from_utf8_lossy(body);
    let msg = serde_json::from_str::<serde_json::Value>(text.trim())
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| "remote provider request failed".to_string());
    unavailable(msg)
}

/// Drive one streaming chat over an already-connected byte stream —
/// generic over the transport so unit tests exercise the whole protocol
/// (HTTP head, chunked decoding, SSE frames) over `tokio::io::duplex`.
fn stream_chat<S>(mut io: S, request: String) -> TokenStream
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    Box::pin(async_stream::stream! {
        if let Err(e) = io.write_all(request.as_bytes()).await {
            yield Err(unavailable(format!("remoted write: {e}")));
            return;
        }
        let head = match read_head(&mut io).await {
            Ok(h) => h,
            Err(e) => {
                yield Err(e);
                return;
            }
        };
        if !head.status_ok {
            // Pre-flight refusals (consent, credentials, upstream) are
            // plain JSON error bodies with a non-200 status.
            match read_body(&mut io, &head).await {
                Ok(body) => yield Err(error_from_body(&body)),
                Err(e) => yield Err(e),
            }
            return;
        }
        if !head.sse {
            // A broker that answers 200 with a plain JSON completion
            // (pre-streaming build): hand the content back whole rather
            // than failing the chat.
            match read_body(&mut io, &head).await {
                Ok(body) => {
                    let text = String::from_utf8_lossy(&body);
                    match serde_json::from_str::<serde_json::Value>(text.trim()) {
                        Ok(v) => {
                            let content = v["choices"][0]["message"]["content"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string();
                            yield Ok(content);
                        }
                        Err(e) => yield Err(unavailable(format!("remoted json: {e}"))),
                    }
                }
                Err(e) => yield Err(e),
            }
            return;
        }

        // The streaming path: chunked HTTP framing → SSE frames →
        // OpenAI chunk deltas, yielded as they arrive.
        let mut decoder = head.chunked.then(ChunkedDecoder::default);
        let mut sse = SseParser::default();
        let mut input = head.rest.clone();
        let mut tmp = [0u8; 4096];
        loop {
            let mut decoded = Vec::new();
            let framing_done = match &mut decoder {
                Some(d) => match d.feed(&input, &mut decoded) {
                    Ok(done) => done,
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                },
                None => {
                    decoded = std::mem::take(&mut input);
                    false
                }
            };
            for data in sse.feed(&decoded) {
                if data.trim() == "[DONE]" {
                    return; // clean end of stream
                }
                let v: serde_json::Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue, // tolerate unknown frames
                };
                if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
                    let msg = err["message"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| err.to_string());
                    yield Err(unavailable(msg));
                    return;
                }
                if let Some(tok) = v["choices"][0]["delta"]["content"].as_str()
                    && !tok.is_empty()
                {
                    yield Ok(tok.to_string());
                }
            }
            if framing_done {
                yield Err(unavailable("remoted stream ended before [DONE]"));
                return;
            }
            match read_some(&mut io, &mut tmp).await {
                Ok(0) => {
                    yield Err(unavailable("remoted stream ended before [DONE]"));
                    return;
                }
                Ok(n) => input = tmp[..n].to_vec(),
                Err(e) => {
                    yield Err(e);
                    return;
                }
            }
        }
    })
}

/// An engine that proxies one provider+model to the broker socket.
pub struct RemoteEngine {
    socket: PathBuf,
    provider: String,
    model: String,
}

impl RemoteEngine {
    fn render_request(&self, messages: &[crate::openai::ChatMessage]) -> String {
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
        })
        .to_string();
        // Scopes declared for this request; the broker checks each against
        // its per-scope consent. A bare prompt carries the `prompt` scope.
        format!(
            "POST /v1/chat/completions HTTP/1.1\r\n\
             Host: lisa-remoted\r\n\
             x-lisa-provider: {}\r\n\
             x-lisa-scopes: prompt\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n{}",
            self.provider,
            body.len(),
            body
        )
    }
}

impl Engine for RemoteEngine {
    fn name(&self) -> &'static str {
        "remote"
    }

    fn generate(&self, req: GenerateRequest) -> TokenStream {
        let socket = self.socket.clone();
        let request = self.render_request(&req.messages);
        Box::pin(async_stream::stream! {
            let io = match UnixStream::connect(&socket).await {
                Ok(s) => s,
                Err(e) => {
                    yield Err(unavailable(format!(
                        "lisa-remoted socket {}: {e} — is the broker running? \
                         (systemctl start lisa-remoted)",
                        socket.display()
                    )));
                    return;
                }
            };
            let mut inner = stream_chat(io, request);
            while let Some(item) = futures::StreamExt::next(&mut inner).await {
                yield item;
            }
        })
    }

    fn embed(&self, _texts: Vec<String>) -> BoxFuture<'static, Result<Vec<Vec<f32>>, EngineError>> {
        Box::pin(async {
            Err(EngineError::Unavailable(
                "remote providers serve chat only; embeddings run on a local model".into(),
            ))
        })
    }
}

/// Wraps any EngineProvider: intercepts `remote:` model names and routes
/// them to the broker; everything else delegates to the local provider.
/// Keeps the api/scheduler/ledger path unchanged for local models.
pub struct RemoteRouter {
    inner: Arc<dyn EngineProvider>,
    socket: PathBuf,
}

impl RemoteRouter {
    pub fn new(inner: Arc<dyn EngineProvider>, socket: PathBuf) -> Self {
        Self { inner, socket }
    }
}

impl EngineProvider for RemoteRouter {
    fn engine_for(
        &self,
        model: Option<&str>,
    ) -> BoxFuture<'_, Result<Arc<dyn Engine>, EngineError>> {
        if let Some(m) = model
            && m.starts_with(REMOTE_PREFIX)
        {
            let parsed = parse(m);
            let socket = self.socket.clone();
            return Box::pin(async move {
                let (provider, model) = parsed.ok_or_else(|| {
                    EngineError::Unavailable(
                        "remote model must be remote:<provider>:<model>, \
                         e.g. remote:huggingface:openai/gpt-oss-120b"
                            .into(),
                    )
                })?;
                Ok(Arc::new(RemoteEngine {
                    socket,
                    provider,
                    model,
                }) as Arc<dyn Engine>)
            });
        }
        self.inner.engine_for(model)
    }

    fn known_models(&self) -> Vec<String> {
        // Remote models are dynamic (provider + arbitrary id); the local
        // set is authoritative for /v1/models. `lisa remote` lists what's
        // configured.
        self.inner.known_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn parses_remote_model_names() {
        assert_eq!(
            parse("remote:huggingface:openai/gpt-oss-120b"),
            Some(("huggingface".into(), "openai/gpt-oss-120b".into()))
        );
        assert_eq!(
            parse("remote:hf:org/model:cheapest"),
            Some(("hf".into(), "org/model:cheapest".into()))
        );
        assert_eq!(parse("remote:openai"), None, "model required");
        assert_eq!(parse("qwen3-8b"), None, "not a remote name");
    }

    #[tokio::test]
    async fn router_delegates_local_names_to_inner() {
        use crate::pool::SingleEngine;
        let inner = Arc::new(SingleEngine {
            engine: Arc::new(crate::engine::StubEngine),
            name: "lisa-system-stub".into(),
        });
        let router = RemoteRouter::new(inner, PathBuf::from("/nonexistent.sock"));
        // A local name resolves to the stub engine (not the broker).
        let engine = router.engine_for(Some("lisa-system-stub")).await.unwrap();
        assert_eq!(engine.name(), "stub");
    }

    #[tokio::test]
    async fn router_routes_remote_names_to_a_remote_engine() {
        use crate::pool::SingleEngine;
        let inner = Arc::new(SingleEngine {
            engine: Arc::new(crate::engine::StubEngine),
            name: "lisa-system-stub".into(),
        });
        let router = RemoteRouter::new(inner, PathBuf::from("/nonexistent.sock"));
        let engine = router
            .engine_for(Some("remote:huggingface:org/model"))
            .await
            .unwrap();
        assert_eq!(engine.name(), "remote");
    }

    #[test]
    fn request_body_asks_the_broker_to_stream() {
        let engine = RemoteEngine {
            socket: PathBuf::from("/x.sock"),
            provider: "openai".into(),
            model: "gpt-x".into(),
        };
        let req = engine.render_request(&[crate::openai::ChatMessage {
            role: "user".into(),
            content: "hi".into(),
        }]);
        assert!(req.contains("x-lisa-provider: openai"));
        assert!(req.contains("x-lisa-scopes: prompt"));
        let body = req.split("\r\n\r\n").nth(1).unwrap();
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(v["stream"], true, "TRUE streaming is requested");
        assert_eq!(v["model"], "gpt-x");
    }

    #[test]
    fn chunked_decoder_handles_split_boundaries() {
        let mut d = ChunkedDecoder::default();
        let mut out = Vec::new();
        // "5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n" fed byte by byte.
        for b in b"5\r\nhello\r\n6\r\n world\r\n" {
            assert!(!d.feed(&[*b], &mut out).unwrap());
        }
        assert_eq!(out, b"hello world");
        assert!(d.feed(b"0\r\n\r\n", &mut out).unwrap(), "terminal chunk");
        assert_eq!(out, b"hello world", "no trailing bytes leak");
    }

    #[test]
    fn sse_parser_reassembles_split_events() {
        let mut p = SseParser::default();
        assert!(p.feed(b"data: {\"a\":").is_empty());
        assert_eq!(
            p.feed(b"1}\n\n: ping\n\ndata: [DONE]\n\n"),
            vec!["{\"a\":1}", "[DONE]"]
        );
    }

    /// Script a broker response and collect what the engine yields.
    async fn run_script(response: &'static [u8]) -> Vec<Result<String, EngineError>> {
        let (client, mut server) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            // Read the request (single frame is fine for tests).
            let _ = server.read(&mut buf).await.unwrap();
            server.write_all(response).await.unwrap();
            server.shutdown().await.ok();
        });
        stream_chat(
            client,
            "POST /v1/chat/completions HTTP/1.1\r\n\r\n{}".into(),
        )
        .collect()
        .await
    }

    fn sse_chunked(frames: &[&str]) -> Vec<u8> {
        let mut resp = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
             Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            .to_vec();
        for f in frames {
            let event = format!("data: {f}\n\n");
            resp.extend_from_slice(format!("{:x}\r\n{}\r\n", event.len(), event).as_bytes());
        }
        resp
    }

    #[tokio::test]
    async fn streams_real_deltas_incrementally() {
        let mut resp = sse_chunked(&[
            r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"hel"},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]);
        resp.extend_from_slice(b"0\r\n\r\n");
        let items = run_script(resp.leak()).await;
        let tokens: Vec<String> = items.into_iter().map(|r| r.unwrap()).collect();
        assert_eq!(tokens, vec!["hel", "lo"], "real deltas, in order");
    }

    #[tokio::test]
    async fn mid_stream_error_frames_surface_as_engine_errors() {
        let mut resp = sse_chunked(&[
            r#"{"choices":[{"index":0,"delta":{"content":"par"},"finish_reason":null}]}"#,
            r#"{"error":{"message":"provider stream stalled"}}"#,
        ]);
        resp.extend_from_slice(b"0\r\n\r\n");
        let items = run_script(resp.leak()).await;
        assert_eq!(items[0].as_ref().unwrap(), "par");
        let err = items[1].as_ref().unwrap_err().to_string();
        assert!(err.contains("provider stream stalled"), "{err}");
        assert_eq!(items.len(), 2, "stream ends after the error");
    }

    #[tokio::test]
    async fn early_eof_is_an_error_not_a_hang() {
        // Stream cut off mid-flight: no [DONE], no terminal chunk.
        let resp = sse_chunked(&[
            r#"{"choices":[{"index":0,"delta":{"content":"hal"},"finish_reason":null}]}"#,
        ]);
        let items = run_script(resp.leak()).await;
        assert_eq!(items[0].as_ref().unwrap(), "hal");
        let err = items[1].as_ref().unwrap_err().to_string();
        assert!(err.contains("ended before [DONE]"), "{err}");
    }

    #[tokio::test]
    async fn broker_refusals_surface_as_errors() {
        let body = r#"{"error":{"message":"scope 'prompt' not consented for offload"}}"#;
        let resp = format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let items = run_script(resp.into_bytes().leak()).await;
        let err = items[0].as_ref().unwrap_err().to_string();
        assert!(err.contains("not consented"), "{err}");
    }

    #[tokio::test]
    async fn plain_json_200_falls_back_to_one_token() {
        // A pre-streaming broker build answers a whole completion.
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"whole reply"}}]}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let items = run_script(resp.into_bytes().leak()).await;
        let tokens: Vec<String> = items.into_iter().map(|r| r.unwrap()).collect();
        assert_eq!(tokens, vec!["whole reply"]);
    }
}
