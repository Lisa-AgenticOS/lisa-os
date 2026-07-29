//! Streaming data plane (ADR-0010, streaming update): incremental SSE
//! parsing of provider byte streams, and translation of the Anthropic
//! Messages event dialect into OpenAI-compatible `chat.completion.chunk`
//! events — so every consumer of the broker sees exactly one wire
//! format, streamed or not. Pure functions/state machines: unit-tested
//! with no network, mirroring `proxy::translate_response` for the
//! non-streaming path.

use crate::registry::Dialect;
use serde_json::{Value, json};

/// Incremental SSE frame parser: feed raw bytes as they arrive, get back
/// complete `data:` payloads. Handles events split across reads, `\n`
/// and `\r\n` line endings, multi-line `data:` fields, and skips
/// comment (`:`) and `event:` lines (Anthropic repeats the event name in
/// the payload's `type` field). Buffers bytes — a multi-byte UTF-8
/// sequence can never be split, because event boundaries are ASCII.
///
/// # Bounded, and linear (issue #103)
///
/// The first version appended every byte to an unbounded `Vec` and
/// rescanned it **from index 0** on each feed. A provider that never
/// emits a blank line therefore cost O(n²) CPU and unbounded memory,
/// and the idle timeout never fired because bytes *were* arriving:
/// 32 MiB of garbage pegged a core for 46 seconds and the buffer was
/// still resident. The provider is by definition an untrusted remote
/// party, and this is the only egress daemon.
///
/// So: a scan cursor, and a ceiling. Past the ceiling the stream is an
/// error rather than a slow death.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    /// How far `event_boundary` has already looked. A byte that was not
    /// a terminator a moment ago is still not one now.
    scanned: usize,
    /// Set once the ceiling is passed; every later feed is refused.
    overflowed: bool,
}

/// The largest SSE frame we will hold before giving up on a provider.
///
/// Real frames are a few KiB; this is orders of magnitude beyond any
/// legitimate one, and small enough that a hostile stream cannot make
/// the daemon interesting to the OOM killer.
pub const MAX_EVENT_BYTES: usize = 512 * 1024;

impl SseParser {
    /// Whether this parser has given up on the stream. A caller that
    /// sees `true` should end the stream with an error rather than keep
    /// feeding it.
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        if self.overflowed {
            return Vec::new();
        }
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some((end, sep)) = event_boundary(&self.buf, self.scanned) {
            let event: Vec<u8> = self.buf.drain(..end + sep).collect();
            self.scanned = 0;
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
        // Nothing left to find in what we have seen; next feed resumes
        // from here instead of from the start. Backing off by two keeps
        // a terminator split across two reads visible.
        self.scanned = self.buf.len().saturating_sub(2);
        if self.buf.len() > MAX_EVENT_BYTES {
            self.overflowed = true;
            // Drop it: holding a hostile buffer after refusing it is the
            // half of the bug that memory-bounding exists to fix.
            self.buf = Vec::new();
            self.scanned = 0;
        }
        out
    }
}

/// First blank line (event terminator) at or after `from`: returns
/// (bytes before the terminating newline pair, separator length to also
/// drain).
fn event_boundary(buf: &[u8], from: usize) -> Option<(usize, usize)> {
    for i in from..buf.len() {
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

/// One normalized item from a provider stream.
#[derive(Debug, PartialEq)]
pub enum StreamItem {
    /// An OpenAI-compatible `chat.completion.chunk`.
    Chunk(Value),
    /// The provider signalled a clean end of stream.
    Done,
    /// A mid-stream provider error.
    Error(String),
}

/// Translates provider SSE `data:` payloads into the OpenAI-compatible
/// chunk shape (§5.1): pass-through for `openai-compat` providers,
/// event mapping for the Anthropic Messages dialect — the streaming
/// mirror of `proxy::translate_response`.
pub struct StreamTranslator {
    dialect: Dialect,
    id: Value,
    model: Value,
    /// Authoritative output-token count when the provider reports one
    /// (Anthropic `message_delta.usage`, OpenAI `usage` chunks).
    pub output_tokens: Option<i64>,
}

impl StreamTranslator {
    pub fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            id: Value::Null,
            model: Value::Null,
            output_tokens: None,
        }
    }

    /// Translate one `data:` payload; `None` for frames that carry
    /// nothing downstream (pings, block start/stop, unknown types).
    pub fn translate(&mut self, data: &str) -> Option<StreamItem> {
        if data.trim() == "[DONE]" {
            return Some(StreamItem::Done);
        }
        let v: Value = serde_json::from_str(data).ok()?;
        match self.dialect {
            Dialect::OpenaiCompat => {
                if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
                    let msg = err["message"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| err.to_string());
                    return Some(StreamItem::Error(msg));
                }
                if let Some(n) = v["usage"]["completion_tokens"].as_i64() {
                    self.output_tokens = Some(n);
                }
                Some(StreamItem::Chunk(v))
            }
            Dialect::AnthropicMessages => self.translate_anthropic(&v),
        }
    }

    fn chunk(&self, delta: Value, finish_reason: Value) -> StreamItem {
        StreamItem::Chunk(json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
        }))
    }

    /// Anthropic SSE events → OpenAI chunks: `message_start` becomes the
    /// role preamble, `content_block_delta` text becomes
    /// `choices[0].delta.content`, `message_delta` carries the mapped
    /// `finish_reason` + usage, `message_stop` ends the stream, `error`
    /// surfaces. `ping`/`content_block_start`/`content_block_stop` and
    /// non-text deltas (tool/thinking) carry nothing downstream.
    fn translate_anthropic(&mut self, v: &Value) -> Option<StreamItem> {
        match v["type"].as_str().unwrap_or_default() {
            "message_start" => {
                self.id = v["message"]["id"].clone();
                self.model = v["message"]["model"].clone();
                Some(self.chunk(json!({"role": "assistant", "content": ""}), Value::Null))
            }
            "content_block_delta" => {
                let text = v["delta"]["text"].as_str()?;
                Some(self.chunk(json!({"content": text}), Value::Null))
            }
            "message_delta" => {
                if let Some(n) = v["usage"]["output_tokens"].as_i64() {
                    self.output_tokens = Some(n);
                }
                let finish = if v["delta"]["stop_reason"] == "max_tokens" {
                    "length"
                } else {
                    "stop"
                };
                Some(self.chunk(json!({}), json!(finish)))
            }
            "message_stop" => Some(StreamItem::Done),
            "error" => Some(StreamItem::Error(
                v["error"]["message"]
                    .as_str()
                    .unwrap_or("provider stream error")
                    .to_string(),
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #103. A provider that never terminates an event used to
    /// cost O(n²) CPU and unbounded memory — 32 MiB took 46 seconds and
    /// was still resident — while the idle timeout stayed quiet because
    /// bytes *were* arriving.
    ///
    /// The assertion is on wall-clock because that is what the bug was.
    /// The bound is deliberately loose (a second for 32 MiB, against 46
    /// observed) so this fails on a regression, not on a slow machine.
    #[test]
    fn a_provider_that_never_ends_an_event_is_cut_off_quickly() {
        let mut p = SseParser::default();
        let chunk = vec![b'A'; 64 * 1024];
        let started = std::time::Instant::now();
        for _ in 0..512 {
            assert!(p.feed(&chunk).is_empty());
        }
        let elapsed = started.elapsed();
        assert!(p.overflowed(), "32 MiB with no event boundary was accepted");
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "quadratic rescan is back: 32 MiB took {elapsed:?}"
        );
    }

    /// Bounded means the bytes are actually released, not merely
    /// noticed. Holding a hostile buffer after refusing it fixes the CPU
    /// half and leaves the memory half.
    #[test]
    fn an_overflowed_parser_drops_what_it_was_holding() {
        let mut p = SseParser::default();
        p.feed(&vec![b'A'; MAX_EVENT_BYTES + 1]);
        assert!(p.overflowed());
        assert_eq!(p.buf.len(), 0, "the hostile buffer is still resident");
        // And it stays refused rather than recovering on the next feed.
        assert!(p.feed(b"data: {}\n\n").is_empty());
    }

    /// The cursor must not skip a terminator that arrives split across
    /// two reads — the exact case a resume-where-you-left-off scan gets
    /// wrong, and the reason the cursor backs off by two.
    #[test]
    fn a_boundary_split_across_reads_is_still_found() {
        for (a, b) in [
            (&b"data: {\"x\":1}\n"[..], &b"\n"[..]),
            (&b"data: {\"x\":1}"[..], &b"\n\n"[..]),
            (&b"data: {\"x\":1}\n\r"[..], &b"\n"[..]),
            (&b"data: {\"x\":1}\n"[..], &b"\r\n"[..]),
        ] {
            let mut p = SseParser::default();
            assert!(p.feed(a).is_empty(), "premature event from {a:?}");
            assert_eq!(
                p.feed(b),
                vec!["{\"x\":1}".to_string()],
                "boundary lost when split as {a:?} + {b:?}"
            );
        }
    }

    /// A long-but-legitimate stream of many small frames must not trip
    /// the ceiling: the bound is on one unterminated frame, not on the
    /// stream.
    #[test]
    fn many_ordinary_frames_never_overflow() {
        let mut p = SseParser::default();
        let mut seen = 0;
        for i in 0..20_000 {
            seen += p.feed(format!("data: {{\"i\":{i}}}\n\n").as_bytes()).len();
        }
        assert_eq!(seen, 20_000);
        assert!(!p.overflowed());
    }

    #[test]
    fn sse_parser_handles_split_events_crlf_and_comments() {
        let mut p = SseParser::default();
        assert!(p.feed(b"data: {\"a\":").is_empty(), "incomplete event");
        assert_eq!(p.feed(b"1}\n\n"), vec!["{\"a\":1}"]);
        // CRLF separators, event: lines, and comments are all handled.
        assert_eq!(
            p.feed(b": keep-alive\r\n\r\nevent: ping\r\ndata: {\"b\":2}\r\n\r\n"),
            vec!["{\"b\":2}"]
        );
        // Two events in one read; multi-line data joins with \n.
        assert_eq!(
            p.feed(b"data: x\ndata: y\n\ndata: [DONE]\n\n"),
            vec!["x\ny", "[DONE]"]
        );
    }

    #[test]
    fn sse_parser_never_splits_utf8() {
        let mut p = SseParser::default();
        let event = "data: {\"t\":\"héllo — ok\"}\n\n".as_bytes();
        // Feed one byte at a time, straight through multi-byte chars.
        let mut got = Vec::new();
        for b in event {
            got.extend(p.feed(&[*b]));
        }
        assert_eq!(got, vec!["{\"t\":\"héllo — ok\"}"]);
    }

    #[test]
    fn openai_chunks_pass_through_and_done_terminates() {
        let mut t = StreamTranslator::new(Dialect::OpenaiCompat);
        let chunk = json!({
            "id": "c1", "object": "chat.completion.chunk",
            "choices": [{"index": 0, "delta": {"content": "hi"}, "finish_reason": null}],
        });
        assert_eq!(
            t.translate(&chunk.to_string()),
            Some(StreamItem::Chunk(chunk))
        );
        // A usage chunk (stream_options include_usage) is authoritative.
        let usage = json!({"choices": [], "usage": {"completion_tokens": 42}});
        assert!(matches!(
            t.translate(&usage.to_string()),
            Some(StreamItem::Chunk(_))
        ));
        assert_eq!(t.output_tokens, Some(42));
        assert_eq!(t.translate("[DONE]"), Some(StreamItem::Done));
    }

    #[test]
    fn openai_error_frames_surface_as_errors() {
        let mut t = StreamTranslator::new(Dialect::OpenaiCompat);
        let err = json!({"error": {"message": "rate limited"}});
        assert_eq!(
            t.translate(&err.to_string()),
            Some(StreamItem::Error("rate limited".into()))
        );
    }

    #[test]
    fn anthropic_events_map_to_openai_chunks() {
        let mut t = StreamTranslator::new(Dialect::AnthropicMessages);
        let start = json!({"type": "message_start", "message":
            {"id": "msg_1", "model": "claude-x", "usage": {"input_tokens": 7}}});
        let Some(StreamItem::Chunk(role)) = t.translate(&start.to_string()) else {
            panic!("message_start should emit the role preamble");
        };
        assert_eq!(role["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(role["id"], "msg_1");
        assert_eq!(role["model"], "claude-x");

        // Pings and block boundaries carry nothing downstream.
        assert_eq!(t.translate(r#"{"type":"ping"}"#), None);
        assert_eq!(
            t.translate(r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#),
            None
        );

        let delta = json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "hel"}});
        let Some(StreamItem::Chunk(c)) = t.translate(&delta.to_string()) else {
            panic!("text delta should emit a content chunk");
        };
        assert_eq!(c["choices"][0]["delta"]["content"], "hel");
        assert_eq!(c["choices"][0]["finish_reason"], Value::Null);
        assert_eq!(c["id"], "msg_1", "id carried from message_start");

        let end = json!({"type": "message_delta",
            "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 3}});
        let Some(StreamItem::Chunk(f)) = t.translate(&end.to_string()) else {
            panic!("message_delta should emit the finish chunk");
        };
        assert_eq!(f["choices"][0]["finish_reason"], "stop");
        assert_eq!(t.output_tokens, Some(3));

        assert_eq!(
            t.translate(r#"{"type":"message_stop"}"#),
            Some(StreamItem::Done)
        );
    }

    #[test]
    fn anthropic_max_tokens_maps_to_length_and_errors_surface() {
        let mut t = StreamTranslator::new(Dialect::AnthropicMessages);
        let end = json!({"type": "message_delta", "delta": {"stop_reason": "max_tokens"}});
        let Some(StreamItem::Chunk(f)) = t.translate(&end.to_string()) else {
            panic!("finish chunk expected");
        };
        assert_eq!(f["choices"][0]["finish_reason"], "length");

        let err = json!({"type": "error",
            "error": {"type": "overloaded_error", "message": "Overloaded"}});
        assert_eq!(
            t.translate(&err.to_string()),
            Some(StreamItem::Error("Overloaded".into()))
        );
    }

    #[test]
    fn non_text_deltas_are_skipped() {
        let mut t = StreamTranslator::new(Dialect::AnthropicMessages);
        // input_json_delta (tool use) has no `text` — nothing downstream.
        let d = json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": "{\"a\""}});
        assert_eq!(t.translate(&d.to_string()), None);
    }
}
