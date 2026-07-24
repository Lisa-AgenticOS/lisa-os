//! Turn — one composed assistant turn, pure (no IO). Given the persona,
//! the memory digest, the skill catalog, the windowed history, and the
//! user input, [`Turn::request_body`] builds the OpenAI chat-completions
//! request body the caller POSTs (ureq, sync — as `cli/lisa` does). The
//! response comes back to the caller, which appends it to the session.
//!
//! A turn can optionally be *guided*: attach a JSON Schema and the
//! request carries `response_format: json_schema`, with [`Turn::grammar`]
//! exposing the GBNF the server compiles (liblisa — the same
//! grammar-constraint guarantee [`liblisa::tasks::Task`] gives).

use crate::session::Message;
use serde_json::{Value, json};

/// One assistant turn, composed. Fields are public for callers that
/// assemble piecemeal; the `with_*` builders chain.
#[derive(Debug, Clone)]
pub struct Turn {
    /// The system persona ("You are Lisa, ...").
    pub persona: String,
    /// This turn's bounded memory digest (`Memory::digest`), injected
    /// into the system prompt under a `## Memory` heading.
    pub memory_digest: String,
    /// The skill catalog lines (`Skill::catalog_line`) — the
    /// progressive-disclosure index, listed under `## Skills`.
    pub skill_catalog: Vec<String>,
    /// The windowed conversation so far (`Session::history`), oldest first.
    pub history: Vec<Message>,
    /// What the user just said.
    pub user_input: String,
    /// Reply cap, mirrored into the request body.
    pub max_tokens: u32,
    /// Optional guided-generation constraint: `(name, JSON Schema)`.
    pub response_format: Option<(String, Value)>,
}

impl Turn {
    pub fn new(persona: impl Into<String>, user_input: impl Into<String>) -> Self {
        Turn {
            persona: persona.into(),
            memory_digest: String::new(),
            skill_catalog: Vec::new(),
            history: Vec::new(),
            user_input: user_input.into(),
            max_tokens: 1024,
            response_format: None,
        }
    }

    pub fn with_digest(mut self, digest: String) -> Self {
        self.memory_digest = digest;
        self
    }

    pub fn with_skills(mut self, catalog: Vec<String>) -> Self {
        self.skill_catalog = catalog;
        self
    }

    pub fn with_history(mut self, history: Vec<Message>) -> Self {
        self.history = history;
        self
    }

    /// Constrain the reply to `schema` (guided generation, §5.1).
    pub fn guided(mut self, name: impl Into<String>, schema: Value) -> Self {
        self.response_format = Some((name.into(), schema));
        self
    }

    /// The system message: persona, then the memory digest and skill
    /// catalog sections (each omitted when empty).
    pub fn system_prompt(&self) -> String {
        let mut s = self.persona.clone();
        if !self.memory_digest.is_empty() {
            s.push_str("\n\n## Memory\n");
            s.push_str(&self.memory_digest);
        }
        if !self.skill_catalog.is_empty() {
            s.push_str("\n\n## Skills\n");
            for line in &self.skill_catalog {
                s.push_str("- ");
                s.push_str(line);
                s.push('\n');
            }
            s.pop();
        }
        s
    }

    /// The `messages` array: system + windowed history + the new user input.
    pub fn messages(&self) -> Vec<Value> {
        let mut msgs = vec![json!({"role": "system", "content": self.system_prompt()})];
        for m in &self.history {
            msgs.push(json!({"role": m.role.as_str(), "content": m.content}));
        }
        msgs.push(json!({"role": "user", "content": self.user_input}));
        msgs
    }

    /// The chat-completions request body. The caller may set
    /// `body["model"]` before POSTing it to `/v1/chat/completions`.
    pub fn request_body(&self) -> Value {
        let mut body = json!({
            "messages": self.messages(),
            "max_tokens": self.max_tokens,
        });
        if let Some((name, schema)) = &self.response_format {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {"name": name, "schema": schema},
            });
        }
        body
    }

    /// The GBNF grammar a guided turn constrains generation to — the same
    /// one the server compiles (liblisa). `None` for unguided turns.
    pub fn grammar(&self) -> Option<Result<String, liblisa::grammar::GrammarError>> {
        self.response_format
            .as_ref()
            .map(|(_, schema)| liblisa::grammar::json_schema_to_gbnf(schema))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Role;

    fn composed() -> Turn {
        Turn::new("You are Lisa, an on-device assistant.", "make it dark")
            .with_digest("- prefers dark theme\n- deploy target is the nuc box".to_string())
            .with_skills(vec![
                "deploy-demo: Deploy the demo site".to_string(),
                "code-review: Review a diff like a senior".to_string(),
            ])
            .with_history(vec![
                Message::new(Role::User, "theme this app"),
                Message::new(Role::Assistant, "which app?"),
            ])
    }

    #[test]
    fn composition_includes_every_pillar() {
        let turn = composed();
        let system = turn.system_prompt();
        assert!(system.starts_with("You are Lisa, an on-device assistant."));
        assert!(system.contains("## Memory\n- prefers dark theme\n- deploy target is the nuc box"));
        assert!(system.contains("## Skills\n- deploy-demo: Deploy the demo site\n- code-review: Review a diff like a senior"));

        let msgs = turn.messages();
        let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["system", "user", "assistant", "user"]);
        assert_eq!(msgs[1]["content"], "theme this app");
        assert_eq!(msgs[2]["content"], "which app?");
        assert_eq!(msgs[3]["content"], "make it dark", "new input is last");

        let body = turn.request_body();
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["messages"].as_array().unwrap().len(), 4);
        assert!(body.get("response_format").is_none(), "unguided by default");
    }

    #[test]
    fn empty_sections_are_omitted_from_the_system_prompt() {
        let turn = Turn::new("You are Lisa.", "hi");
        assert_eq!(turn.system_prompt(), "You are Lisa.");
        let msgs = turn.messages();
        assert_eq!(msgs.len(), 2, "no history: just system + user");
    }

    #[test]
    fn guided_turn_carries_a_compilable_schema() {
        let turn = Turn::new("You are Lisa.", "milk, eggs").guided(
            "groceries",
            json!({
                "type": "object",
                "properties": {"items": {"type": "array", "items": {"type": "string", "maxLength": 40}}},
                "required": ["items"]
            }),
        );
        let body = turn.request_body();
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["name"], "groceries");
        // The schema compiles to a grammar via liblisa (guaranteed-valid output).
        let grammar = turn.grammar().expect("guided").expect("compiles");
        assert!(grammar.starts_with("root ::="), "grammar: {grammar}");
        assert!(
            Turn::new("p", "u").grammar().is_none(),
            "unguided → no grammar"
        );
    }
}
