//! The Agent Bus tool family for the one agent loop (ADR-0025, issue #59).
//!
//! `forge-harness` ships the *workspace* family — jailed files and
//! allowlisted commands inside one project. This is the second family:
//! the tools apps register with `lisa-agentd`, discovered at runtime from
//! `dev.lisaos.Agent1.ListTools` and offered to the model through the
//! same `ToolProvider` seam. One loop, many families, instead of a
//! separate ad-hoc router per surface.
//!
//! # Read tier only, and why that is a product decision rather than the
//! # guardrail
//!
//! This family offers **only `Read`-tier tools**. Those resolve `Silent`
//! with a trusted chain, so they need no confirmation and nothing about
//! them depends on a human being present.
//!
//! Write and destructive tools are withheld because of the residual gap
//! recorded in ADR-0035 §4 and issue #55: the process hosting the model
//! is currently also the process that raises the confirmation dialog, so
//! for a call it originates itself, requester and approver are the same
//! peer. A confirmation in that arrangement is the model asking itself
//! for permission. Until that split lands, the honest move is not to
//! offer the tools that would need one.
//!
//! **The filter here is not what makes that safe.** `lisa-agentd` resolves
//! the tier itself, from the manifest, on the far side of a D-Bus call
//! the model cannot forge (ADR-0030: anything reachable from inside is
//! not a guardrail). If a write tool ever reached [`execute`], agentd
//! would park it and we would report that it needs a human — see
//! [`outcome_for`]. The filter is the *product* decision to not advertise
//! what cannot be used; agentd is the thing that enforces it.

use crate::agent::wire_name;
use forge_harness::{ToolCall, ToolOutcome, ToolProvider, ToolSpec};
use serde_json::Value;
use std::collections::HashMap;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

const DEST: &str = "dev.lisaos.Agent1";
const PATH: &str = "/dev/lisaos/Agent1";
const IFACE: &str = "dev.lisaos.Agent1";

/// One bus tool as the loop sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct BusTool {
    /// Owning app, as `RequestCall` wants it.
    pub app_id: String,
    /// Tool name within that app.
    pub tool: String,
    /// The flattened name the MODEL sees and calls back with. Bus ids
    /// are `app.lisaos.notes::create_note`, which is not a legal OpenAI
    /// tool name.
    pub wire: String,
    pub description: String,
    pub input_schema: Value,
}

/// Pure: `ListTools` JSON → the Read-tier tools, wire names assigned.
///
/// Rows missing `app_id`/`name` are skipped rather than failing the whole
/// catalog: one malformed manifest on the system must not cost the model
/// every other app's tools.
///
/// A row with **no `tier` at all is dropped**, not defaulted to Read. A
/// tool whose sensitivity we cannot read is not a tool we know is safe to
/// call unattended, and defaulting the unknown to the permissive value is
/// how a fail-open lands in a security boundary.
pub fn read_tier_tools(raw: &str) -> anyhow::Result<Vec<BusTool>> {
    let rows: Vec<Value> =
        serde_json::from_str(raw).map_err(|e| anyhow::anyhow!("parsing ListTools JSON: {e}"))?;
    Ok(rows
        .iter()
        .filter(|r| r.get("tier").and_then(Value::as_str) == Some("read"))
        .filter_map(|r| {
            let app_id = r.get("app_id")?.as_str()?.to_string();
            let tool = r.get("name")?.as_str()?.to_string();
            Some(BusTool {
                wire: wire_name(&app_id, &tool),
                description: r
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                input_schema: r
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
                app_id,
                tool,
            })
        })
        .collect())
}

/// Pure: an agentd disposition → what the loop should put in the history.
///
/// The one that matters is `confirm-chip` / `confirm-modal`. **This must
/// never answer its own confirmation.** Calling `Confirm` here would make
/// the model both requester and approver, which is precisely the hole
/// issue #55 exists to close — and it would be worse than the current
/// gap, because it would be automatic. The call stays parked and the
/// model is told, in the tool result, that a person has to act.
///
/// Failures come back as *result text* rather than as errors that end the
/// run: the loop's contract is that a tool failure is something the model
/// can see and route around, and a denied tool is not a crashed harness.
pub fn outcome_for(disposition: &str, detail: &str, label: &str) -> ToolOutcome {
    match disposition {
        // Bus tools act on app state, never on the forge project, so the
        // verifier has nothing to re-check: `mutated` stays false.
        "executed" => ToolOutcome::ok(detail.to_string(), false),
        "failed" => ToolOutcome::err(format!("{label} failed: {detail}")),
        "denied" => ToolOutcome::err(format!("{label} denied by policy: {detail}")),
        "confirm-chip" | "confirm-modal" => ToolOutcome::err(format!(
            "{label} needs a person to confirm it and none is present; the call is \
             parked, not done. Do not retry it — retrying cannot make it approved. \
             Tell the user what you were trying to do and let them run it."
        )),
        other => ToolOutcome::err(format!("{label}: unknown disposition {other:?}: {detail}")),
    }
}

/// The Read-tier Agent Bus tools, discovered once at construction.
///
/// The catalog is a snapshot: an app that registers a tool mid-run is not
/// picked up. That is deliberate for now — the tool list is handed to the
/// backend at the start of the loop and changing it underneath a
/// conversation means the model has a spec for a tool that no longer
/// exists.
pub struct AgentBusTools {
    conn: Connection,
    tools: Vec<BusTool>,
}

impl AgentBusTools {
    /// Connect and fetch the catalog. Returns `Ok(None)` when agentd is
    /// simply not there — a dev host with no session bus is a normal
    /// state, not an error, and the loop still runs with the families
    /// that did load.
    pub fn discover() -> anyhow::Result<Option<Self>> {
        let Ok(conn) = Connection::session() else {
            return Ok(None);
        };
        let Ok(reply) = conn.call_method(Some(DEST), PATH, Some(IFACE), "ListTools", &()) else {
            return Ok(None);
        };
        let raw: String = reply.body().deserialize()?;
        Ok(Some(Self {
            tools: read_tier_tools(&raw)?,
            conn,
        }))
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    fn find(&self, wire: &str) -> Option<&BusTool> {
        self.tools.iter().find(|t| t.wire == wire)
    }
}

impl ToolProvider for AgentBusTools {
    fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .map(|t| {
                ToolSpec::new(
                    t.wire.clone(),
                    t.description.clone(),
                    t.input_schema.clone(),
                )
            })
            .collect()
    }

    fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let Some(tool) = self.find(&call.name) else {
            // Unreachable through the loop's dispatch, which only routes
            // names this provider advertised — but a provider that
            // assumes that and is wrong executes the WRONG tool.
            return ToolOutcome::err(format!("no bus tool named `{}`", call.name));
        };
        let label = format!("{}::{}", tool.app_id, tool.tool);

        // `actor: assistant` and provenance `["user"]`: this call chain
        // began with a person typing. An event-triggered chain is NOT
        // this and must not borrow this provenance (ADR-0036 §1) — when
        // event triggers land, the chain has to be threaded in from the
        // trigger rather than asserted here.
        let options: HashMap<String, OwnedValue> = match (
            OwnedValue::try_from(zbus::zvariant::Value::from("assistant")),
            OwnedValue::try_from(zbus::zvariant::Value::from(vec!["user"])),
        ) {
            (Ok(actor), Ok(prov)) => HashMap::from([
                ("actor".to_string(), actor),
                ("provenance".to_string(), prov),
            ]),
            _ => return ToolOutcome::err("could not build the call options"),
        };

        let reply = self.conn.call_method(
            Some(DEST),
            PATH,
            Some(IFACE),
            "RequestCall",
            &(
                tool.app_id.as_str(),
                tool.tool.as_str(),
                call.args.to_string(),
                options,
            ),
        );
        let reply = match reply {
            Ok(r) => r,
            Err(e) => return ToolOutcome::err(format!("{label}: RequestCall failed: {e}")),
        };
        match reply.body().deserialize::<(u64, String, String)>() {
            Ok((_id, disposition, detail)) => outcome_for(&disposition, &detail, &label),
            Err(e) => ToolOutcome::err(format!("{label}: unreadable reply: {e}")),
        }
    }
}

/// `lisa assist "<what you want>"` — the multi-turn assistant on the
/// harness (ADR-0025), as opposed to `lisa do`, which routes a single
/// utterance to exactly one tool call and stops.
///
/// The difference is the loop: this can search, read the result, decide
/// it needs something else, and search again, which is the whole reason
/// ADR-0025 asks for one agent loop rather than a router per surface.
///
/// The verifier is `None` — there is no project to check. The loop
/// therefore ends when the model says it is done or the turn budget runs
/// out, and `project` only exists because the verifier would have used
/// it.
pub fn assist_cmd(
    utterance: &str,
    url: &str,
    model: Option<String>,
    max_turns: usize,
) -> anyhow::Result<()> {
    let Some(bus) = AgentBusTools::discover()? else {
        anyhow::bail!(
            "no session bus — `lisa assist` needs a desktop session with lisa-agentd running"
        );
    };
    if bus.is_empty() {
        anyhow::bail!(
            "lisa-agentd registered no read-tier tools, so there is nothing to work with. \
             `lisa tools` lists what it does know about."
        );
    }
    eprintln!("assist: {} read-tier bus tool(s)", bus.len());

    let mut backend = forge_harness::OpenAiBackend {
        url: url.to_string(),
        model,
    };
    // Same reasoning as the forge loop (#54, #129): a loop that acts on
    // your behalf is exactly the thing that must be on the record, and a
    // machine with an unwritable Ledger refuses to run rather than
    // acting off it.
    let ledger = std::sync::Arc::new(lisa_ledger::Ledger::open(
        lisa_ledger::Ledger::default_path(),
    )?);
    let config = forge_harness::AgentConfig {
        max_turns,
        verifier: forge_harness::Verifier::None,
        ..forge_harness::AgentConfig::new(ledger)
    };
    let mut observe = |ev: forge_harness::AgentEvent| {
        use forge_harness::AgentEvent as E;
        match ev {
            E::Turn { n, max } => eprintln!("[turn {n}/{max}]"),
            E::Call { name, detail } => eprintln!("  · {name} {detail}"),
            E::CallResult { ok: false, chars } => {
                eprintln!("    ! tool error ({chars} chars)")
            }
            _ => {}
        }
    };
    let providers: [&dyn ToolProvider; 1] = [&bus];
    let project = std::env::current_dir()?;
    let report = forge_harness::forge_agent_with_tools(
        utterance,
        &project,
        &mut backend,
        &config,
        &providers,
        &mut observe,
    )?;
    println!("{}", report.summary);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = r#"[
      {"app_id":"app.lisaos.notes","name":"search_notes","tier":"read",
       "description":"Search notes","undoable":false,
       "input_schema":{"type":"object","properties":{"q":{"type":"string"}}}},
      {"app_id":"app.lisaos.notes","name":"create_note","tier":"write",
       "description":"Create a note","undoable":true,
       "input_schema":{"type":"object","properties":{}}},
      {"app_id":"app.lisaos.files","name":"delete_file","tier":"destructive",
       "description":"Delete","undoable":false,
       "input_schema":{"type":"object","properties":{}}},
      {"app_id":"app.lisaos.notes","name":"list_notes","tier":"read",
       "description":"List notes","undoable":false,
       "input_schema":{"type":"object","properties":{}}}
    ]"#;

    #[test]
    fn only_read_tier_tools_are_offered() {
        let tools = read_tier_tools(CATALOG).unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.tool.as_str()).collect();
        assert_eq!(names, vec!["search_notes", "list_notes"]);
        // The point of the slice: nothing that would need a confirmation
        // is advertised to the model at all.
        assert!(!names.contains(&"create_note"));
        assert!(!names.contains(&"delete_file"));
    }

    #[test]
    fn a_tool_with_no_tier_is_dropped_not_assumed_read() {
        let raw = r#"[{"app_id":"a.b","name":"mystery",
                       "input_schema":{"type":"object"}}]"#;
        assert!(read_tier_tools(raw).unwrap().is_empty());
        // Nor is an unrecognised tier waved through.
        let odd = r#"[{"app_id":"a.b","name":"x","tier":"readonly",
                       "input_schema":{"type":"object"}}]"#;
        assert!(read_tier_tools(odd).unwrap().is_empty());
    }

    #[test]
    fn wire_names_are_openai_legal_and_map_back() {
        let tools = read_tier_tools(CATALOG).unwrap();
        for t in &tools {
            assert!(t.wire.len() <= 64, "{} too long", t.wire);
            assert!(
                t.wire
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "{} has an illegal character",
                t.wire
            );
        }
        // Distinct tools keep distinct wire names — a collision would
        // silently route one tool's call to another.
        let a = &tools[0];
        let b = &tools[1];
        assert_ne!(a.wire, b.wire);
    }

    #[test]
    fn the_input_schema_survives_verbatim() {
        // The backend is grammar-constrained against this; a dropped or
        // defaulted schema means unconstrained arguments.
        let tools = read_tier_tools(CATALOG).unwrap();
        let search = tools.iter().find(|t| t.tool == "search_notes").unwrap();
        assert_eq!(search.input_schema["properties"]["q"]["type"], "string");
    }

    #[test]
    fn a_missing_schema_becomes_an_empty_object_not_a_dropped_tool() {
        let raw = r#"[{"app_id":"a.b","name":"x","tier":"read"}]"#;
        let tools = read_tier_tools(raw).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].input_schema["type"], "object");
    }

    #[test]
    fn a_malformed_row_does_not_cost_the_whole_catalog() {
        let raw = r#"[{"tier":"read","name":"no_app"},
                      {"app_id":"a.b","name":"good","tier":"read"}]"#;
        let tools = read_tier_tools(raw).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool, "good");
    }

    #[test]
    fn executed_is_the_only_disposition_that_is_not_an_error() {
        let ok = outcome_for("executed", "3 notes", "app::search");
        assert_eq!(ok.text, "3 notes");
        assert!(!ok.mutated, "bus tools never touch the forge project");

        for bad in ["failed", "denied", "weird"] {
            assert!(
                outcome_for(bad, "d", "app::t").text.starts_with("error:"),
                "{bad} should be an error"
            );
        }
    }

    /// The load-bearing one: a parked confirmation must never be answered
    /// from in here. If this ever returns something that reads as success,
    /// the model has approved its own privileged call.
    #[test]
    fn a_confirmation_is_reported_never_answered() {
        for d in ["confirm-chip", "confirm-modal"] {
            let out = outcome_for(d, "", "app.lisaos.notes::create_note");
            assert!(out.text.starts_with("error:"), "{d} must not read as done");
            assert!(
                out.text.contains("person"),
                "{d} must say a human is required"
            );
            assert!(!out.mutated);
        }
    }
}
