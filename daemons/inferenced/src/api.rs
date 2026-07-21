//! HTTP surface: OpenAI-compatible endpoints on loopback (`docs/PLAN.md`
//! §5.1). In production, per-app identity via SO_PEERCRED → portal grants
//! attaches here (M2); guided generation (JSON Schema → GBNF via
//! `liblisa::grammar`) is threaded through to the engine in M1.

use crate::engine::{Engine, GenerateRequest};
use crate::openai::*;
use crate::scheduler::{Priority, Scheduler};
use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use futures::StreamExt;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<dyn Engine>,
    pub scheduler: Arc<Scheduler>,
    pub model_name: String,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/embeddings", post(embeddings))
        .with_state(state)
}

async fn embeddings(State(state): State<AppState>, Json(req): Json<serde_json::Value>) -> Response {
    // OpenAI shape: input is a string or an array of strings.
    let texts: Vec<String> = match &req["input"] {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(xs) => xs
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    };
    if texts.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": {"message": "input must be a string or array of strings"}})),
        )
            .into_response();
    }
    match state.engine.embed(texts).await {
        Ok(vectors) => Json(serde_json::json!({
            "object": "list",
            "model": state.model_name,
            "data": vectors.iter().enumerate().map(|(i, v)| serde_json::json!({
                "object": "embedding",
                "index": i,
                "embedding": v,
            })).collect::<Vec<_>>(),
            "usage": {"prompt_tokens": 0, "total_tokens": 0},
        }))
        .into_response(),
        Err(e) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": {"message": e.to_string()}})),
        )
            .into_response(),
    }
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "engine": state.engine.name(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn models(State(state): State<AppState>) -> Json<ModelList> {
    Json(ModelList {
        object: "list",
        data: vec![ModelInfo {
            id: state.model_name.clone(),
            object: "model",
            created: unix_now(),
            owned_by: "lisa",
        }],
    })
}

async fn chat_completions(
    State(state): State<AppState>,
    Json(req): Json<ChatCompletionRequest>,
) -> Response {
    let model = req
        .model
        .clone()
        .unwrap_or_else(|| state.model_name.clone());
    let id = format!("chatcmpl-lisa-{}", unix_now());
    let created = unix_now();

    // Guided generation: JSON Schema → GBNF, enforced by the sampler.
    let grammar = match &req.response_format {
        Some(rf) if rf["type"] == "json_schema" => {
            match liblisa::grammar::json_schema_to_gbnf(&rf["json_schema"]["schema"]) {
                Ok(g) => Some(g),
                Err(e) => {
                    return (
                        axum::http::StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": {
                            "message": format!("response_format schema not supported: {e}"),
                            "type": "invalid_request_error",
                        }})),
                    )
                        .into_response();
                }
            }
        }
        _ => None,
    };

    let priority = Priority::parse(req.lisa_priority.as_deref());
    let guided = grammar.is_some();
    let gen_req = GenerateRequest {
        messages: req.messages,
        grammar,
        max_tokens: req.max_tokens,
    };

    if req.stream {
        let stream = state.engine.generate(gen_req);
        let stream = state.scheduler.admit(priority, stream).await;
        let chunk_id = id.clone();
        let chunk_model = model.clone();
        let sse = async_stream::stream! {
            // Role preamble chunk, per OpenAI streaming convention.
            yield sse_json(&ChatCompletionChunk {
                id: chunk_id.clone(),
                object: "chat.completion.chunk",
                created,
                model: chunk_model.clone(),
                choices: vec![ChunkChoice {
                    index: 0,
                    delta: Delta { role: Some("assistant"), content: None },
                    finish_reason: None,
                }],
            });
            let mut stream = stream;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(token) => yield sse_json(&ChatCompletionChunk {
                        id: chunk_id.clone(),
                        object: "chat.completion.chunk",
                        created,
                        model: chunk_model.clone(),
                        choices: vec![ChunkChoice {
                            index: 0,
                            delta: Delta { role: None, content: Some(token) },
                            finish_reason: None,
                        }],
                    }),
                    Err(e) => {
                        yield Ok(Event::default()
                            .data(serde_json::json!({"error": {"message": e.to_string()}}).to_string()));
                        break;
                    }
                }
            }
            yield sse_json(&ChatCompletionChunk {
                id: chunk_id.clone(),
                object: "chat.completion.chunk",
                created,
                model: chunk_model.clone(),
                choices: vec![ChunkChoice {
                    index: 0,
                    delta: Delta::default(),
                    finish_reason: Some("stop"),
                }],
            });
            yield Ok(Event::default().data("[DONE]"));
        };
        return Sse::new(sse)
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    // Non-streaming: aggregate the token stream. Guided requests get one
    // server-side re-sample if the output isn't valid JSON (a truncated
    // constrained generation must not reach the caller — structured
    // output is the contract, §5.1/§5.6).
    let attempts = if guided { 2 } else { 1 };
    let mut content = String::new();
    for attempt in 0..attempts {
        let stream = state.engine.generate(gen_req.clone());
        let stream = state.scheduler.admit(priority, stream).await;
        let tokens: Vec<Result<String, _>> = stream.collect().await;
        content.clear();
        let mut failed = None;
        for t in tokens {
            match t {
                Ok(tok) => content.push_str(&tok),
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = failed {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": {"message": e.to_string()}})),
            )
                .into_response();
        }
        if !guided || serde_json::from_str::<serde_json::Value>(&content).is_ok() {
            break;
        }
        tracing::warn!(attempt, "guided output was not valid JSON; re-sampling");
    }
    let completion_tokens = content.split_whitespace().count() as u32;
    Json(ChatCompletionResponse {
        id,
        object: "chat.completion",
        created,
        model,
        choices: vec![Choice {
            index: 0,
            message: ChatMessage {
                role: "assistant".into(),
                content,
            },
            finish_reason: "stop",
        }],
        usage: Usage {
            prompt_tokens: 0,
            completion_tokens,
            total_tokens: completion_tokens,
        },
    })
    .into_response()
}

fn sse_json<T: serde::Serialize>(value: &T) -> Result<Event, std::convert::Infallible> {
    Ok(Event::default()
        .data(serde_json::to_string(value).expect("wire types serialize infallibly")))
}
