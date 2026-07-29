//! Agent Bus client — `lisa do` / `tools` / `call` / `undo` (PLAN §5.4,
//! ADR-0013).
//!
//! `do` is the whole intent pipeline in one verb: fetch the tool catalog
//! from `dev.lisaos.Agent1`, run the liblisa intent router (stage 1: pick a
//! tool, grammar-guaranteed; stage 2: fill its args against the tool's own
//! schema), then hand the call to the bus, where tiers, provenance, undo,
//! and the Ledger apply. The CLI is a *trusted-user* surface: provenance is
//! `["user"]`, so read-tier calls execute silently and write/destructive
//! park for confirmation exactly as the tier table says.

use anyhow::{Context, anyhow, bail};
use bus_tools::wire_name;
use liblisa::intent::{self, ToolInfo};
use serde_json::Value;
use std::collections::HashMap;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

const DEST: &str = "dev.lisaos.Agent1";
const PATH: &str = "/dev/lisaos/Agent1";
const IFACE: &str = "dev.lisaos.Agent1";

/// Session-bus connection to lisa-agentd, with a CLI-appropriate error.
fn connect() -> anyhow::Result<Connection> {
    Connection::session().context(
        "connecting to the session bus (is this a desktop session with \
         lisa-agentd running?)",
    )
}

fn call_string(
    conn: &Connection,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> anyhow::Result<String> {
    let reply = conn
        .call_method(Some(DEST), PATH, Some(IFACE), method, body)
        .with_context(|| format!("calling {IFACE}.{method} (is lisa-agentd running?)"))?;
    Ok(reply.body().deserialize::<String>()?)
}

/// The catalog as the intent router wants it.
fn catalog(conn: &Connection) -> anyhow::Result<Vec<ToolInfo>> {
    let raw = call_string(conn, "ListTools", &())?;
    parse_catalog(&raw)
}

/// Pure: ListTools JSON → ToolInfo list (unit-tested without a bus).
pub fn parse_catalog(raw: &str) -> anyhow::Result<Vec<ToolInfo>> {
    let rows: Vec<Value> = serde_json::from_str(raw).context("parsing ListTools JSON")?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(ToolInfo {
                app_id: r.get("app_id")?.as_str()?.to_string(),
                tool: r.get("name")?.as_str()?.to_string(),
                description: r
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                input_schema: r
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
            })
        })
        .collect())
}

/// Run a guided-generation task against the inference endpoint and parse
/// the structured JSON the grammar guarantees.
fn run_task(
    url: &str,
    model: Option<&str>,
    task: &liblisa::tasks::Task,
    input: &str,
) -> anyhow::Result<Value> {
    let mut body = task.request(input);
    if let Some(m) = model {
        body["model"] = Value::String(m.to_string());
    }
    let endpoint = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
    let mut response = ureq::post(&endpoint)
        .send_json(&body)
        .with_context(|| format!("intent routing via {endpoint} (is lisa-inferenced up?)"))?;
    let reply: Value = response.body_mut().read_json()?;
    let content = reply["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow!("no content in inference reply"))?;
    serde_json::from_str(content).context("parsing guided-generation output")
}

/// Route by NATIVE TOOL CALLING — the path Claude/GPT are built for, and
/// the only one available to them: the grammar router below constrains
/// the sampler, which no remote provider exposes (a `remote:` model hits
/// it as a 503). The bus catalog goes over as OpenAI `tools`, and the
/// model's `tool_calls` reply *is* the routed intent, arguments included,
/// so one round-trip replaces the local two-stage dance.
fn route_via_tool_calling(
    url: &str,
    model: Option<&str>,
    tools: &[ToolInfo],
    utterance: &str,
) -> anyhow::Result<Option<intent::IntentCall>> {
    let specs: Vec<Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": wire_name(&t.app_id, &t.tool),
                    "description": t.description,
                    "parameters": if t.input_schema.is_object() {
                        t.input_schema.clone()
                    } else {
                        serde_json::json!({"type": "object", "properties": {}})
                    },
                },
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "messages": [
            {"role": "system", "content":
             "You route one user request to at most one tool on the Lisa Agent Bus. \
              Call the tool that fulfils it, with arguments taken from the request. \
              If nothing fits, reply in words and call no tool."},
            {"role": "user", "content": utterance},
        ],
        "tools": specs,
        "stream": false,
    });
    if let Some(m) = model {
        body["model"] = Value::String(m.to_string());
    }
    let endpoint = format!("{}/v1/chat/completions", url.trim_end_matches('/'));
    let mut response = ureq::post(&endpoint)
        .send_json(&body)
        .with_context(|| format!("tool-call routing via {endpoint}"))?;
    let reply: Value = response.body_mut().read_json()?;
    let Some(call) = reply["choices"][0]["message"]["tool_calls"]
        .as_array()
        .and_then(|c| c.first())
    else {
        return Ok(None); // model declined to call anything
    };
    let name = call["function"]["name"]
        .as_str()
        .ok_or_else(|| anyhow!("tool_call without a function name"))?;
    let chosen = tools
        .iter()
        .find(|t| wire_name(&t.app_id, &t.tool) == name)
        .ok_or_else(|| anyhow!("model called `{name}`, which is not on the bus"))?;
    // Arguments arrive as a JSON *string* per the OpenAI wire format.
    let args: Value = match call["function"]["arguments"].as_str() {
        Some(s) if !s.trim().is_empty() => {
            serde_json::from_str(s).context("parsing tool_call arguments")?
        }
        _ => serde_json::json!({}),
    };
    Ok(Some(intent::IntentCall::from_user(
        &intent::Choice {
            app_id: chosen.app_id.clone(),
            tool: chosen.tool.clone(),
            confidence: 1.0,
        },
        args,
    )))
}

/// `lisa do "<utterance>"` — route and (unless dry-run) execute.
pub fn do_cmd(
    utterance: &str,
    url: &str,
    model: Option<&str>,
    dry_run: bool,
    yes: bool,
) -> anyhow::Result<()> {
    let conn = connect()?;
    let tools = catalog(&conn)?;
    if tools.is_empty() {
        bail!("no tools on the Agent Bus — install an app manifest first (lisa tools)");
    }

    // Capability, not preference: remote providers expose tool calling
    // but not sampler grammars, so they take the native path; local
    // llamas take the grammar router, whose output is *guaranteed*
    // well-formed — which small models badly need.
    if model.is_some_and(|m| m.starts_with("remote:")) {
        let Some(call) = route_via_tool_calling(url, model, &tools, utterance)? else {
            println!("nothing on the bus fits that request (intent: none)");
            return Ok(());
        };
        println!("→ {}::{} args {}", call.app_id, call.tool, call.args);
        if dry_run {
            return Ok(());
        }
        return request_and_confirm(&conn, &call.app_id, &call.tool, &call.args, yes);
    }

    // Stage 1: pick the tool (grammar-guaranteed one of the catalog).
    let router = intent::router(&tools);
    let out = run_task(url, model, &router, utterance)?;
    let Some(choice) = intent::parse_choice(&out)? else {
        println!("nothing on the bus fits that request (intent: none)");
        return Ok(());
    };
    let tool = tools
        .iter()
        .find(|t| t.app_id == choice.app_id && t.tool == choice.tool)
        .ok_or_else(|| anyhow!("router chose an unknown tool"))?;

    // Stage 2: fill that tool's args against its own schema.
    let args = match intent::arg_filler(tool) {
        Some(filler) => run_task(url, model, &filler, utterance)?,
        None => serde_json::json!({}),
    };
    let call = intent::IntentCall::from_user(&choice, args);

    println!(
        "→ {}::{} (confidence {:.2}) args {}",
        call.app_id, call.tool, choice.confidence, call.args
    );
    if dry_run {
        return Ok(());
    }
    request_and_confirm(&conn, &call.app_id, &call.tool, &call.args, yes)
}

/// `lisa tools` — the registered catalog, one line per tool.
pub fn tools_cmd() -> anyhow::Result<()> {
    let conn = connect()?;
    let tools = catalog(&conn)?;
    if tools.is_empty() {
        println!("no tools registered (install app manifests under /usr/share/lisa/manifests)");
        return Ok(());
    }
    for t in &tools {
        println!("{}::{}  —  {}", t.app_id, t.tool, t.description);
    }
    Ok(())
}

/// `lisa call <app_id> <tool> [args-json]` — direct, no model in the loop.
pub fn call_cmd(app_id: &str, tool: &str, args: Option<&str>, yes: bool) -> anyhow::Result<()> {
    let args: Value = match args {
        Some(a) => serde_json::from_str(a).context("args must be a JSON object")?,
        None => serde_json::json!({}),
    };
    let conn = connect()?;
    request_and_confirm(&conn, app_id, tool, &args, yes)
}

/// `lisa undo` — revert the last undoable action.
pub fn undo_cmd() -> anyhow::Result<()> {
    let conn = connect()?;
    let report = call_string(&conn, "Undo", &())?;
    println!("{report}");
    Ok(())
}

/// RequestCall + the confirmation round-trip. Chip-level confirmations
/// prompt on the terminal (or auto-approve with `--yes`); modal-level ones
/// are refused here — they belong to the shell's consent UI.
fn request_and_confirm(
    conn: &Connection,
    app_id: &str,
    tool: &str,
    args: &Value,
    yes: bool,
) -> anyhow::Result<()> {
    let options: HashMap<String, OwnedValue> = HashMap::from([
        (
            "actor".to_string(),
            OwnedValue::try_from(zbus::zvariant::Value::from("cli"))?,
        ),
        (
            "provenance".to_string(),
            OwnedValue::try_from(zbus::zvariant::Value::from(vec!["user"]))?,
        ),
    ]);
    let reply = conn
        .call_method(
            Some(DEST),
            PATH,
            Some(IFACE),
            "RequestCall",
            &(app_id, tool, args.to_string(), options),
        )
        .context("RequestCall failed (is lisa-agentd running?)")?;
    let (call_id, disposition, detail): (u64, String, String) = reply.body().deserialize()?;

    match disposition.as_str() {
        "executed" => {
            println!("✓ executed: {detail}");
            Ok(())
        }
        "failed" => bail!("tool failed: {detail}"),
        "denied" => bail!("denied by policy: {detail}"),
        "confirm-chip" => {
            let approve = yes || prompt_yes(&format!("{app_id}::{tool} {args} — proceed? [y/N] "))?;
            let reply = conn.call_method(
                Some(DEST),
                PATH,
                Some(IFACE),
                "Confirm",
                &(call_id, approve),
            )?;
            let (status, detail): (String, String) = reply.body().deserialize()?;
            if status == "executed" {
                println!("✓ executed: {detail}");
                Ok(())
            } else {
                bail!("{status}: {detail}")
            }
        }
        "confirm-modal" => bail!(
            "this action needs the desktop consent dialog (destructive tier) — \
             use the overlay, or re-run the underlying tool from its app"
        ),
        other => bail!("unknown disposition {other:?}: {detail}"),
    }
}

pub fn prompt_yes(prompt: &str) -> anyhow::Result<bool> {
    use std::io::Write as _;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_are_openai_legal_and_round_trip() {
        let tools = parse_catalog(
            r#"[{"app_id":"app.lisaos.notes","name":"create_note","description":"d"},
                {"app_id":"app.lisaos.notes","name":"list_notes","description":"d"},
                {"app_id":"dev.lisaos.files-x","name":"read","description":"d"}]"#,
        )
        .unwrap();
        let names: Vec<String> = tools
            .iter()
            .map(|t| wire_name(&t.app_id, &t.tool))
            .collect();
        for n in &names {
            assert!(
                n.len() <= 64
                    && n.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "`{n}` is not a legal OpenAI tool name"
            );
        }
        // Distinct tools must never collide — the reply is mapped back by name.
        let unique: std::collections::HashSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "wire names collided: {names:?}");
        // And each maps back to exactly the tool it came from.
        for (t, n) in tools.iter().zip(&names) {
            let back = tools
                .iter()
                .find(|c| &wire_name(&c.app_id, &c.tool) == n)
                .unwrap();
            assert_eq!((&back.app_id, &back.tool), (&t.app_id, &t.tool));
        }
    }

    #[test]
    fn parse_catalog_maps_list_tools_json() {
        let raw = r#"[
            {"app_id":"app.lisaos.notes","name":"create_note","tier":"write",
             "description":"Create a note","undoable":true,
             "input_schema":{"type":"object","properties":{"title":{"type":"string"}},
                             "required":["title"]}},
            {"app_id":"app.lisaos.notes","name":"list_notes","tier":"read",
             "description":"List notes","undoable":false,
             "input_schema":{"type":"object","properties":{}}}
        ]"#;
        let tools = parse_catalog(raw).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].id(), "app.lisaos.notes::create_note");
        assert!(tools[0].takes_args());
        assert!(!tools[1].takes_args());
        // The catalog drives a compilable router grammar end to end.
        let task = intent::router(&tools);
        assert!(task.grammar().is_ok());
    }

    #[test]
    fn parse_catalog_tolerates_missing_schema() {
        // Older agentd without input_schema in ListTools: args default to
        // an empty object schema (tool still callable, just arg-less).
        let raw = r#"[{"app_id":"a.b","name":"t","tier":"read",
                       "description":"","undoable":false}]"#;
        let tools = parse_catalog(raw).unwrap();
        assert_eq!(tools.len(), 1);
        assert!(!tools[0].takes_args());
    }
}
