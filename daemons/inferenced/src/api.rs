//! HTTP surface: OpenAI-compatible endpoints on loopback (`docs/PLAN.md`
//! §5.1). In production, per-app identity via SO_PEERCRED → portal grants
//! attaches here (M2); guided generation (JSON Schema → GBNF via
//! `liblisa::grammar`) is threaded through to the engine in M1.

use crate::engine::GenerateRequest;
use crate::openai::*;
use crate::pool::EngineProvider;
use crate::scheduler::{Priority, Scheduler};
use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use futures::StreamExt;
use lisa_ledger::{Event as LedgerEvent, Ledger, preview_of};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub engines: Arc<dyn EngineProvider>,
    pub scheduler: Arc<Scheduler>,
    /// Reported by /health; "stub" or "llama".
    pub engine_kind: String,
    pub model_name: String,
    pub ledger: Arc<Ledger>,
}

fn engine_error_response(e: crate::engine::EngineError) -> Response {
    (
        axum::http::StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": {"message": e.to_string()}})),
    )
        .into_response()
}

/// Dataflow rule 4 (PLAN §4): the ledger entry precedes the action —
/// if the ledger cannot record it, the action must not happen.
fn ledger_gate(ledger: &Ledger, event: &LedgerEvent) -> Result<i64, Box<Response>> {
    ledger.append(event).map_err(|e| {
        Box::new(
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": {
                    "message": format!("refusing to run without a ledger entry: {e}"),
                }})),
            )
                .into_response(),
        )
    })
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
    let started = std::time::Instant::now();
    let entry_id = match ledger_gate(
        &state.ledger,
        &LedgerEvent {
            kind: "inference.embed".into(),
            app_id: "host".into(),
            model: state.model_name.clone(),
            input_hash: blake3::hash(texts.join("\n").as_bytes())
                .to_hex()
                .to_string(),
            preview: preview_of(&texts.join(" | ")),
            status: "started".into(),
            ..Default::default()
        },
    ) {
        Ok(id) => id,
        Err(resp) => return *resp,
    };
    let engine = match state.engines.engine_for(req["model"].as_str()).await {
        Ok(e) => e,
        Err(e) => return engine_error_response(e),
    };
    match engine.embed(texts).await {
        Ok(vectors) => {
            let _ = state.ledger.append(&LedgerEvent {
                kind: "inference.complete".into(),
                app_id: "host".into(),
                model: state.model_name.clone(),
                status: "ok".into(),
                ref_id: Some(entry_id),
                duration_ms: started.elapsed().as_millis() as i64,
                ..Default::default()
            });
            Json(serde_json::json!({
                "object": "list",
                "model": state.model_name,
                "data": vectors.iter().enumerate().map(|(i, v)| serde_json::json!({
                    "object": "embedding",
                    "index": i,
                    "embedding": v,
                })).collect::<Vec<_>>(),
                "usage": {"prompt_tokens": 0, "total_tokens": 0},
            }))
            .into_response()
        }
        Err(e) => {
            let _ = state.ledger.append(&LedgerEvent {
                kind: "inference.complete".into(),
                app_id: "host".into(),
                model: state.model_name.clone(),
                status: "error".into(),
                detail: e.to_string(),
                ref_id: Some(entry_id),
                duration_ms: started.elapsed().as_millis() as i64,
                ..Default::default()
            });
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": {"message": e.to_string()}})),
            )
                .into_response()
        }
    }
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "engine": state.engine_kind,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// The tools/tool-calling lane: ledger the exchange, resolve the engine,
/// and pass the OpenAI-compat body through verbatim (stream forced off).
async fn chat_completions_tools(state: AppState, mut raw: serde_json::Value) -> Response {
    if raw
        .get("stream")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": {
                "message": "streaming with tools is not supported yet — send stream:false",
                "type": "invalid_request_error",
            }})),
        )
            .into_response();
    }
    let model_req = raw
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let model = model_req
        .clone()
        .unwrap_or_else(|| state.model_name.clone());
    let priority = Priority::parse(raw.get("lisa_priority").and_then(serde_json::Value::as_str));
    // Hash covers the tool schemas too (#36) — they steer the output as
    // much as the messages do, and the ledger hash is the audit anchor.
    let prompt_all = format!(
        "{}{}",
        raw.get("messages")
            .map(std::string::ToString::to_string)
            .unwrap_or_default(),
        raw.get("tools")
            .map(std::string::ToString::to_string)
            .unwrap_or_default()
    );
    let started_at = std::time::Instant::now();
    let entry_id = match ledger_gate(
        &state.ledger,
        &LedgerEvent {
            kind: "inference.generate".into(),
            app_id: "host".into(),
            model: model.clone(),
            input_hash: blake3::hash(prompt_all.as_bytes()).to_hex().to_string(),
            preview: preview_of(&prompt_all),
            status: "started".into(),
            detail: "tools".into(),
            ..Default::default()
        },
    ) {
        Ok(id) => id,
        Err(resp) => return *resp,
    };
    let engine = match state.engines.engine_for(model_req.as_deref()).await {
        Ok(e) => e,
        Err(e) => return engine_error_response(e),
    };
    raw["stream"] = serde_json::Value::Bool(false);
    // Same slot pool and preemption contract as the token lane (#34):
    // a tool turn must not run outside the scheduler's view.
    match state
        .scheduler
        .admit_future(priority, engine.raw_chat(raw))
        .await
    {
        Ok(child) => {
            let output_tokens = child
                .pointer("/usage/completion_tokens")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            let _ = state.ledger.append(&LedgerEvent {
                kind: "inference.complete".into(),
                app_id: "host".into(),
                model,
                status: "ok".into(),
                ref_id: Some(entry_id),
                output_tokens,
                duration_ms: started_at.elapsed().as_millis() as i64,
                ..Default::default()
            });
            Json(child).into_response()
        }
        Err(e) => {
            let _ = state.ledger.append(&LedgerEvent {
                kind: "inference.complete".into(),
                app_id: "host".into(),
                model,
                status: "error".into(),
                // Child error bodies can be huge — cap like the other
                // lanes do (#36).
                detail: e.to_string().chars().take(200).collect(),
                ref_id: Some(entry_id),
                duration_ms: started_at.elapsed().as_millis() as i64,
                ..Default::default()
            });
            // Engine failure here is unavailability, not a bad route:
            // 503 like the typed lane, never 404 (#36).
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": {"message": e.to_string()}})),
            )
                .into_response()
        }
    }
}

async fn models(State(state): State<AppState>) -> Json<ModelList> {
    Json(ModelList {
        object: "list",
        data: state
            .engines
            .known_models()
            .into_iter()
            .map(|id| ModelInfo {
                id,
                object: "model",
                created: unix_now(),
                owned_by: "lisa",
            })
            .collect(),
    })
}

async fn chat_completions(
    State(state): State<AppState>,
    Json(raw): Json<serde_json::Value>,
) -> Response {
    // Tool-calling requests take the raw passthrough lane: tool turns
    // carry null content and extra roles the typed ChatMessage cannot
    // represent, and the child's tool_calls must reach the client
    // verbatim (found on the M4 rig — forge got plain text back).
    // Routing keys on a NON-EMPTY tools array (#35): OpenAI SDKs send
    // "tools": null / [] on plain requests, and those must stay on the
    // typed lane with its guided-generation and scheduler guarantees.
    match raw.get("tools") {
        Some(serde_json::Value::Array(tools)) if !tools.is_empty() => {
            return chat_completions_tools(state, raw).await;
        }
        None | Some(serde_json::Value::Null) | Some(serde_json::Value::Array(_)) => {}
        Some(_) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": {
                    "message": "tools must be an array of tool definitions",
                    "type": "invalid_request_error",
                }})),
            )
                .into_response();
        }
    }
    let req: ChatCompletionRequest = match serde_json::from_value(raw) {
        Ok(r) => r,
        Err(e) => {
            return (
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": {
                    "message": format!("invalid request: {e}"),
                    "type": "invalid_request_error",
                }})),
            )
                .into_response();
        }
    };
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

    let prompt_all = gen_req
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let started_at = std::time::Instant::now();
    let entry_id = match ledger_gate(
        &state.ledger,
        &LedgerEvent {
            kind: "inference.generate".into(),
            app_id: "host".into(),
            model: model.clone(),
            input_hash: blake3::hash(prompt_all.as_bytes()).to_hex().to_string(),
            preview: preview_of(&prompt_all),
            status: "started".into(),
            detail: if guided {
                "guided".into()
            } else {
                String::new()
            },
            ..Default::default()
        },
    ) {
        Ok(id) => id,
        Err(resp) => return *resp,
    };

    let engine = match state.engines.engine_for(req.model.as_deref()).await {
        Ok(e) => e,
        Err(e) => return engine_error_response(e),
    };

    if req.stream {
        let stream = engine.generate(gen_req);
        let stream = state.scheduler.admit(priority, stream).await;
        let chunk_id = id.clone();
        let chunk_model = model.clone();
        let ledger = Arc::clone(&state.ledger);
        let sse = async_stream::stream! {
            let mut streamed_tokens: i64 = 0;
            let mut stream_status = String::from("ok");
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
                    Ok(token) => {
                        streamed_tokens += 1;
                        yield sse_json(&ChatCompletionChunk {
                            id: chunk_id.clone(),
                            object: "chat.completion.chunk",
                            created,
                            model: chunk_model.clone(),
                            choices: vec![ChunkChoice {
                                index: 0,
                                delta: Delta { role: None, content: Some(token) },
                                finish_reason: None,
                            }],
                        })
                    }
                    Err(e) => {
                        stream_status = if matches!(e, crate::engine::EngineError::Preempted) {
                            "preempted".into()
                        } else {
                            "error".into()
                        };
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
            let _ = ledger.append(&LedgerEvent {
                kind: "inference.complete".into(),
                app_id: "host".into(),
                model: chunk_model.clone(),
                status: stream_status.clone(),
                ref_id: Some(entry_id),
                output_tokens: streamed_tokens,
                duration_ms: started_at.elapsed().as_millis() as i64,
                ..Default::default()
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
        let stream = engine.generate(gen_req.clone());
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
            let _ = state.ledger.append(&LedgerEvent {
                kind: "inference.complete".into(),
                app_id: "host".into(),
                model: model.clone(),
                status: "error".into(),
                detail: e.to_string(),
                ref_id: Some(entry_id),
                duration_ms: started_at.elapsed().as_millis() as i64,
                ..Default::default()
            });
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
    let _ = state.ledger.append(&LedgerEvent {
        kind: "inference.complete".into(),
        app_id: "host".into(),
        model: model.clone(),
        status: "ok".into(),
        ref_id: Some(entry_id),
        output_tokens: i64::from(completion_tokens),
        duration_ms: started_at.elapsed().as_millis() as i64,
        ..Default::default()
    });
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
