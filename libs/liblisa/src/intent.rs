//! NL→intent router — Siri-style App-Intents on Lisa's substrate (ADR-0013).
//!
//! Turns a natural-language utterance into a typed, grammar-*guaranteed*
//! choice of one tool from a catalog, with filled arguments. The caller then
//! hands the result to the Agent Bus (`dev.lisaos.Agent1.RequestCall`), where
//! confirmation tiers, provenance escalation, the undo journal, and the
//! Ledger apply — so every intent is trust-checked by construction.
//!
//! Two stages, because the M0 grammar subset ([`crate::grammar`]) has no
//! `oneOf`, so a single output object cannot grammar-constrain *per-tool*
//! argument shapes:
//!
//! 1. [`router`] — pick exactly one tool id (an `enum`) or `"none"`.
//! 2. [`arg_filler`] — fill *that* tool's arguments against its own
//!    `input_schema` (a normal object schema → a valid grammar).
//!
//! Both stages are fully guided-generation-constrained, which is strictly
//! stronger than prompt-only (Hermes-style) tool calling on small local
//! models. liblisa stays HTTP-free: the caller runs each [`Task`] against an
//! OpenAI-compatible endpoint and feeds the parsed output back here.

use crate::tasks::{Task, extract};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The sentinel intent for "no tool fits" — keeps the router honest rather
/// than forcing a wrong call.
pub const NO_INTENT: &str = "none";

/// One tool the router may choose: a projection of the Agent Bus's
/// `ToolDecl` (liblisa stays agentd-free — this is a plain local type).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    pub app_id: String,
    pub tool: String,
    pub description: String,
    /// JSON Schema for the tool's arguments (as declared in its manifest).
    pub input_schema: Value,
}

impl ToolInfo {
    /// The catalog id the router selects by: `"<app_id>::<tool>"`.
    pub fn id(&self) -> String {
        format!("{}::{}", self.app_id, self.tool)
    }

    /// Whether this tool takes any arguments (drives whether stage 2 runs).
    pub fn takes_args(&self) -> bool {
        self.input_schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|p| !p.is_empty())
    }
}

/// Stage 1: the router task. Its output schema is `{intent, confidence}`
/// where `intent` is an `enum` over the catalog ids plus [`NO_INTENT`], so
/// guided generation guarantees the model returns a real choice.
pub fn router(tools: &[ToolInfo]) -> Task {
    let mut catalog = String::new();
    for t in tools {
        catalog.push_str(&format!("- {} — {}\n", t.id(), t.description));
    }
    let mut ids: Vec<Value> = tools.iter().map(|t| Value::String(t.id())).collect();
    ids.push(Value::String(NO_INTENT.to_string()));

    Task {
        name: "intent_router".into(),
        system: format!(
            "You are Lisa's intent router. Given the user's request and the tools \
             below, choose EXACTLY ONE tool id that fulfils the request, or \
             \"{NO_INTENT}\" if none fits. Prefer \"{NO_INTENT}\" over a wrong \
             guess. Reply ONLY as the JSON object: intent (one of the ids or \
             \"{NO_INTENT}\"), confidence (0..1).\n\nTools:\n{catalog}"
        ),
        schema: json!({
            "type": "object",
            "properties": {
                "intent": {"type": "string", "enum": ids},
                "confidence": {"type": "number"}
            },
            "required": ["intent", "confidence"]
        }),
    }
}

/// The tool the router picked (stage 1 result).
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub app_id: String,
    pub tool: String,
    pub confidence: f64,
}

/// Parse the router's structured output. `Ok(None)` = [`NO_INTENT`]
/// (nothing to do); `Ok(Some)` = a chosen `(app_id, tool)`. Guided
/// generation should make malformed output impossible, but callers still
/// get a clean error rather than a panic.
pub fn parse_choice(output: &Value) -> Result<Option<Choice>, IntentError> {
    let intent = output
        .get("intent")
        .and_then(Value::as_str)
        .ok_or(IntentError::MissingField("intent"))?;
    if intent == NO_INTENT {
        return Ok(None);
    }
    let (app_id, tool) = intent
        .split_once("::")
        .ok_or(IntentError::MalformedIntentId)?;
    if app_id.is_empty() || tool.is_empty() {
        return Err(IntentError::MalformedIntentId);
    }
    Ok(Some(Choice {
        app_id: app_id.to_string(),
        tool: tool.to_string(),
        confidence: output
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
    }))
}

/// Stage 2: fill the chosen tool's arguments from the utterance, constrained
/// by the tool's own `input_schema`. Returns `None` when the tool takes no
/// arguments (skip the second model call; args are `{}`).
pub fn arg_filler(tool: &ToolInfo) -> Option<Task> {
    if !tool.takes_args() {
        return None;
    }
    Some(extract(
        &format!("{}-args", sanitize(&tool.id())),
        &format!(
            "Fill the arguments for the tool \"{}\" ({}) from the user's request.",
            tool.id(),
            tool.description
        ),
        tool.input_schema.clone(),
    ))
}

/// A bus-ready call assembled from the two stages. `chain` is the provenance
/// trail; a direct user utterance seeds `["user"]` (trusted). Serializable so
/// a CLI/overlay can pass it straight to `Agent1.RequestCall`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentCall {
    pub app_id: String,
    pub tool: String,
    pub args: Value,
    pub chain: Vec<String>,
}

impl IntentCall {
    /// Assemble from the stage-1 choice and stage-2 args (`{}` when the tool
    /// takes none). Provenance is seeded as a direct user request.
    pub fn from_user(choice: &Choice, args: Value) -> Self {
        IntentCall {
            app_id: choice.app_id.clone(),
            tool: choice.tool.clone(),
            args: if args.is_object() { args } else { json!({}) },
            chain: vec!["user".to_string()],
        }
    }
}

/// Errors parsing router output. Guided generation prevents these in
/// practice; they exist so a misbehaving endpoint fails cleanly.
#[derive(Debug, PartialEq, Eq)]
pub enum IntentError {
    MissingField(&'static str),
    MalformedIntentId,
}

impl std::fmt::Display for IntentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntentError::MissingField(k) => write!(f, "router output missing field `{k}`"),
            IntentError::MalformedIntentId => {
                write!(f, "intent id is not `<app_id>::<tool>`")
            }
        }
    }
}
impl std::error::Error for IntentError {}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<ToolInfo> {
        vec![
            ToolInfo {
                app_id: "app.lisaos.notes".into(),
                tool: "create".into(),
                description: "Create a note with a title and body".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "maxLength": 80},
                        "body": {"type": "string", "maxLength": 400}
                    },
                    "required": ["title"]
                }),
            },
            ToolInfo {
                app_id: "app.lisaos.lights".into(),
                tool: "all_off".into(),
                description: "Turn every light off".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
        ]
    }

    #[test]
    fn router_output_schema_compiles_to_a_grammar() {
        // The whole point: the router's choice is grammar-guaranteed valid.
        let task = router(&catalog());
        let g = task.grammar().expect("router schema must compile");
        assert!(g.starts_with("root ::="), "grammar: {g}");
        // The enum carries every tool id plus the `none` sentinel.
        assert!(g.contains("app.lisaos.notes::create"), "grammar: {g}");
        assert!(g.contains("app.lisaos.lights::all_off"), "grammar: {g}");
        assert!(g.contains("none"), "grammar: {g}");
        // Both catalog ids are listed in the prompt for the model.
        assert!(
            task.system
                .contains("app.lisaos.notes::create — Create a note")
        );
    }

    #[test]
    fn parse_choice_reads_a_real_choice() {
        let out = json!({"intent": "app.lisaos.notes::create", "confidence": 0.9});
        let choice = parse_choice(&out).unwrap().expect("a choice");
        assert_eq!(choice.app_id, "app.lisaos.notes");
        assert_eq!(choice.tool, "create");
        assert!((choice.confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn parse_choice_handles_none_and_malformed() {
        assert_eq!(
            parse_choice(&json!({"intent": "none", "confidence": 0.1})).unwrap(),
            None
        );
        assert_eq!(
            parse_choice(&json!({"confidence": 0.5})),
            Err(IntentError::MissingField("intent"))
        );
        assert_eq!(
            parse_choice(&json!({"intent": "notoolsep", "confidence": 0.5})),
            Err(IntentError::MalformedIntentId)
        );
    }

    #[test]
    fn arg_filler_only_for_tools_with_args_and_its_schema_compiles() {
        let tools = catalog();
        // notes.create takes args → a fill task whose grammar compiles.
        let filler = arg_filler(&tools[0]).expect("notes.create takes args");
        assert!(filler.grammar().is_ok(), "arg grammar must compile");
        assert!(filler.system.contains("app.lisaos.notes::create"));
        // lights.all_off takes none → no second model call.
        assert!(arg_filler(&tools[1]).is_none());
    }

    #[test]
    fn intent_call_assembles_bus_ready_with_user_provenance() {
        let choice = Choice {
            app_id: "app.lisaos.notes".into(),
            tool: "create".into(),
            confidence: 0.8,
        };
        let call = IntentCall::from_user(&choice, json!({"title": "milk"}));
        assert_eq!(call.app_id, "app.lisaos.notes");
        assert_eq!(call.tool, "create");
        assert_eq!(call.args["title"], "milk");
        assert_eq!(call.chain, vec!["user".to_string()]);
        // Non-object args are coerced to an empty object (never garbage).
        let call2 = IntentCall::from_user(&choice, json!("oops"));
        assert_eq!(call2.args, json!({}));
    }
}
