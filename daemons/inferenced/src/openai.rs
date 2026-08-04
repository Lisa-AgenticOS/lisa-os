//! OpenAI-compatible wire types — the zero-dependency path for existing
//! apps and Electron/web/CLI tools (`docs/PLAN.md` §5.1, §5.6). Only the
//! fields we serve are modeled; unknown request fields are ignored, which
//! is what stock OpenAI clients expect.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Deserialize)]
pub struct ChatCompletionRequest {
    #[serde(default)]
    pub model: Option<String>,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    /// OpenAI structured-outputs shape: {"type":"json_schema",
    /// "json_schema":{"name":..., "schema":{...}}} — compiled to GBNF via
    /// liblisa and enforced by the sampler (guided generation).
    #[serde(default)]
    pub response_format: Option<serde_json::Value>,
    /// Lisa extension: "interactive" (default) | "background". Background
    /// requests are preempted by interactive ones (PLAN §5.1).
    #[serde(default)]
    pub lisa_priority: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// What a message carries: plain text, or OpenAI-style content PARTS —
/// the shape multimodal models take images and audio in.
///
/// The parts are `serde_json::Value` on purpose: Lisa does not
/// re-model every provider's part schema (`image_url`, `input_audio`,
/// and whatever arrives next), it PASSES THEM THROUGH. Re-modelling
/// would mean a new release every time a provider adds a modality,
/// and a silent drop for anything we had not modelled yet — the
/// failure that is worst here, because a dropped image still gets a
/// confident answer about an image nobody saw.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<serde_json::Value>),
}

impl Content {
    /// The text a TEXT-ONLY engine can act on. Non-text parts are named,
    /// never silently dropped — see `has_non_text`, which is what
    /// actually stops a blind model from answering about a picture.
    pub fn text(&self) -> String {
        match self {
            Content::Text(s) => s.clone(),
            Content::Parts(parts) => parts
                .iter()
                .map(|p| match p["type"].as_str() {
                    Some("text") => p["text"].as_str().unwrap_or_default().to_string(),
                    Some(other) => format!("[{other}]"),
                    None => String::new(),
                })
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// Does this carry anything a text-only model cannot see?
    pub fn has_non_text(&self) -> bool {
        match self {
            Content::Text(_) => false,
            Content::Parts(parts) => parts.iter().any(part_is_non_text),
        }
    }
}

/// What a text-only engine says about content it cannot see.
///
/// ONE sentence, shared by every lane that has to say it. It was written
/// out by hand in the typed lane and nowhere else, which is how #236
/// happened: the tools lane had no copy of the rule *and* no copy of the
/// text, so nothing about it was even visibly missing.
pub const TEXT_ONLY_REFUSAL: &str = "this model reads text only — pick a multimodal model \
     (e.g. a remote: provider) for images or audio";

/// Is this content part something other than text?
///
/// A part with **no `type` at all** counts as non-text, deliberately.
/// `Content::text()` renders such a part as the empty string and then
/// filters it out — i.e. it drops it silently — which is the exact
/// failure the modality refusal exists to prevent: a confident answer
/// about something nobody looked at. Refusing is the fail-closed
/// reading, and a part with no `type` is malformed for every provider
/// this daemon forwards to anyway.
fn part_is_non_text(part: &serde_json::Value) -> bool {
    part["type"].as_str() != Some("text")
}

/// Does a RAW OpenAI-compat body carry anything a text-only model cannot
/// see (#236)?
///
/// The passthrough lane never builds a [`ChatCompletionRequest`] — that
/// is the whole point of it, the body goes to the engine verbatim — so
/// [`Content::has_non_text`] could not see what it was carrying. This is
/// the same question asked of the untyped shape.
pub fn body_has_non_text(body: &serde_json::Value) -> bool {
    body["messages"].as_array().is_some_and(|messages| {
        messages.iter().any(|m| {
            m["content"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(part_is_non_text))
        })
    })
}

impl From<String> for Content {
    fn from(s: String) -> Self {
        Content::Text(s)
    }
}

impl From<&str> for Content {
    fn from(s: &str) -> Self {
        Content::Text(s.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Content,
}

impl ChatMessage {
    pub fn text(&self) -> String {
        self.content.text()
    }
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Debug, Serialize)]
pub struct Choice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: &'static str,
}

#[derive(Debug, Serialize, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
}

#[derive(Debug, Serialize)]
pub struct ChunkChoice {
    pub index: u32,
    pub delta: Delta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<&'static str>,
}

#[derive(Debug, Serialize, Default)]
pub struct Delta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ModelList {
    pub object: &'static str,
    pub data: Vec<ModelInfo>,
}

#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub owned_by: &'static str,
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(v: serde_json::Value) -> ChatMessage {
        serde_json::from_value(v).expect("a message the wire could carry")
    }

    #[test]
    fn a_plain_string_content_still_parses_and_round_trips() {
        let m = parts(serde_json::json!({"role": "user", "content": "hi"}));
        assert_eq!(m.text(), "hi");
        assert!(!m.content.has_non_text());
        // The wire shape must not change for text-only callers — every
        // provider and the local engine take a bare string.
        let back = serde_json::to_value(&m).unwrap();
        assert_eq!(back["content"], "hi");
    }

    #[test]
    fn content_parts_survive_the_round_trip_verbatim() {
        let m = parts(serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "what is this?"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAA"}},
            ],
        }));
        let back = serde_json::to_value(&m).unwrap();
        assert_eq!(
            back["content"][1]["image_url"]["url"], "data:image/png;base64,AAA",
            "parts pass through untouched — Lisa does not re-model provider schemas"
        );
    }

    #[test]
    fn text_flattening_names_what_it_cannot_show() {
        let m = parts(serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "describe"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAA"}},
                {"type": "input_audio", "input_audio": {"data": "AAA", "format": "wav"}},
            ],
        }));
        assert_eq!(m.text(), "describe\n[image_url]\n[input_audio]");
        assert!(
            m.content.has_non_text(),
            "the flag a text-only engine refuses on — flattening alone would let a \
             blind model answer confidently about a picture nobody saw"
        );
    }

    #[test]
    fn a_parts_message_of_only_text_is_not_multimodal() {
        let m = parts(serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "still just words"}],
        }));
        assert!(!m.content.has_non_text());
        assert_eq!(m.text(), "still just words");
    }

    /// Issue #236. The typed lane deserializes into [`Content`]; the
    /// passthrough lane never does, and asked the question of nothing at
    /// all. Both lanes must now answer the same way about the same
    /// bytes, so the two predicates are checked against each other here
    /// rather than trusted to agree.
    #[test]
    fn both_lanes_answer_the_same_way_about_the_same_message() {
        for content in [
            serde_json::json!("just words"),
            serde_json::json!([{"type": "text", "text": "just words"}]),
            serde_json::json!([
                {"type": "text", "text": "what is this?"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAA"}},
            ]),
            serde_json::json!([{"type": "input_audio", "input_audio": {"data": "AAA"}}]),
        ] {
            let typed = parts(serde_json::json!({"role": "user", "content": content}))
                .content
                .has_non_text();
            let raw = body_has_non_text(
                &serde_json::json!({"messages": [{"role": "user", "content": content}]}),
            );
            assert_eq!(typed, raw, "the two lanes disagree about {content}");
        }
    }

    /// A part with no `type` is treated as non-text, and that is the
    /// deliberate reading rather than an accident of the comparison.
    /// `Content::text()` renders such a part as nothing and filters it
    /// away — a silent drop, which is precisely the failure the refusal
    /// exists to prevent.
    #[test]
    fn a_part_that_does_not_say_what_it_is_counts_as_unreadable() {
        let body = serde_json::json!({
            "messages": [{"role": "user", "content": [
                {"image_url": {"url": "data:image/png;base64,AAA"}},
            ]}],
        });
        assert!(
            body_has_non_text(&body),
            "a typeless part slipped past as though it were text"
        );
    }

    /// The control: the predicate must not simply answer yes. A body of
    /// plain strings — every text-only conversation ever — is untouched,
    /// and so is a body with no messages at all.
    #[test]
    fn an_ordinary_text_conversation_is_not_called_multimodal() {
        assert!(!body_has_non_text(&serde_json::json!({
            "messages": [
                {"role": "system", "content": "you are lisa"},
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "1", "type": "function",
                     "function": {"name": "read_file", "arguments": "{}"}},
                ]},
                {"role": "tool", "tool_call_id": "1", "content": "file contents"},
            ],
            "tools": [{"type": "function", "function": {"name": "read_file"}}],
        })));
        assert!(!body_has_non_text(&serde_json::json!({})));
        assert!(!body_has_non_text(&serde_json::json!({"messages": []})));
    }

    /// The refusal is one sentence in one place, and it has to say both
    /// what is wrong and what to do — "unsupported" sends a person
    /// hunting through settings for a switch that does not exist.
    #[test]
    fn the_refusal_says_what_to_do_about_it() {
        assert!(TEXT_ONLY_REFUSAL.contains("reads text only"));
        assert!(
            TEXT_ONLY_REFUSAL.contains("multimodal"),
            "the refusal does not name the way out: {TEXT_ONLY_REFUSAL}"
        );
    }
}
