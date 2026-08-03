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
            Content::Parts(parts) => parts
                .iter()
                .any(|p| p["type"].as_str().is_some_and(|t| t != "text")),
        }
    }
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
}
