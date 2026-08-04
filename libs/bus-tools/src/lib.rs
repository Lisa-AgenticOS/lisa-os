//! The Agent Bus tool family for the one agent loop (ADR-0025).

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
///
/// # Why the Read filter is load-bearing, not a placeholder (#216)
///
/// This is the ONLY thing keeping write-tier tools away from the model:
/// `navigate`, `click`, `fill`, `create_note`, `archive_message` are all
/// registered with agentd and all reachable by anything that can open
/// the socket. Widening it looks like a one-word change and is not.
///
/// Measured on the reference machine, 2026-08-04, against the live
/// daemons: a write-tier `RequestCall` does park as `confirm-modal` with
/// `escalated: true`, so the tier machinery is real — but
/// `dev.lisaos.Consent1` has no owner (it is activatable, and agentd
/// asks with `GetNameOwner`, which does not activate), so
/// `consent_role()` answers `Absent`, and `Absent` is the headless
/// fallback that lets the REQUESTER answer its own call. The probe's own
/// connection called `Confirm(id, true)` and agentd dispatched.
///
/// So if this filter were widened today, the last thing between the
/// model and a privileged call would be [`outcome_for`] declining to
/// call `Confirm` — a decision inside the process the model drives.
/// CLAUDE.md rule 6a: reachable from inside is not a guardrail. The
/// filter comes off when a consent surface is *running* and
/// *independent*, proven on a seated session — not before.
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
        // Refused, not denied (#251). The distinction is worth spelling
        // out to the model for the same reason it exists on the wire: a
        // denial is a person saying no this time, a refusal is an action
        // that will never be available to an agent. Telling it to stop —
        // and that the capability belongs to the person — is what keeps
        // a refusal from becoming a retry loop, which is how a rare,
        // meaningful dialog turns into one people dismiss.
        "refused" => ToolOutcome::err(format!(
            "{label} is refused and always will be: {detail}. This is not something \
             Lisa will do through a tool. Do not retry it and do not look for \
             another tool that does the same thing. Tell the user what you were \
             trying to do; if they want it, they can do it themselves."
        )),
        "confirm-chip" | "confirm-modal" => ToolOutcome::err(format!(
            "{label} needs a person to confirm it and none is present; the call is \
             parked, not done. Do not retry it — retrying cannot make it approved. \
             Tell the user what you were trying to do and let them run it."
        )),
        other => ToolOutcome::err(format!("{label}: unknown disposition {other:?}: {detail}")),
    }
}

/// Pure: does an executed call's detail JSON say its content came from
/// the web? The browser's MCP server tags every result it emits
/// (`apps/surfer/lib/mcp-protocol.js`); agentd passes the tool result
/// through in `detail.result`.
///
/// Only the well-formed spelling counts — a page that *contains* the
/// text `"provenance":"web"` in its body text does not taint via string
/// match, because this parses the JSON rather than searching it.
pub fn result_is_web_tagged(detail: &str) -> bool {
    serde_json::from_str::<Value>(detail)
        .ok()
        .and_then(|d| {
            d.get("result")
                .and_then(|r| r.get("provenance"))
                .and_then(Value::as_str)
                .map(|p| p == "web")
        })
        .unwrap_or(false)
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
    /// What woke this run up, as a provenance tag (ADR-0036 §1). Every
    /// call carries it, so agentd's `resolve()` can tell "a person typed
    /// this" from "a schedule fired" without the loop remembering to say
    /// so at each call site.
    trigger: &'static str,
    /// Set the moment any tool result arrives tagged `provenance: "web"`
    /// (#146 Phase 4). From then on every call this provider makes
    /// carries `web` in its chain, agentd's `resolve()` escalates
    /// anything privileged, and a page cannot quietly steer a write. The
    /// taint is one-way for the life of the loop: the model has read the
    /// content, and nothing un-reads it.
    web_tainted: std::cell::Cell<bool>,
}

impl AgentBusTools {
    /// Connect and fetch the catalog. Returns `Ok(None)` when agentd is
    /// simply not there — a dev host with no session bus is a normal
    /// state, not an error, and the loop still runs with the families
    /// that did load.
    /// Discover with the default `user` trigger — a person is typing.
    pub fn discover() -> anyhow::Result<Option<Self>> {
        Self::discover_with_trigger("user")
    }

    /// Discover for a run woken by something other than a person.
    /// `trigger` is the caller's RESOLVED class, never what a message
    /// claimed — see `lisa-harnessd`'s `Trigger::resolve`.
    pub fn discover_with_trigger(trigger: &'static str) -> anyhow::Result<Option<Self>> {
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
            trigger,
            web_tainted: std::cell::Cell::new(false),
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

        // `actor: assistant`, and the chain reflects what this loop has
        // CONSUMED, not just who typed: it starts `["user"]` and gains
        // `"web"` after any web-tagged result (#146 Phase 4). An
        // event-triggered chain is NOT this and must not borrow this
        // provenance (ADR-0036 §1).
        // The chain says where this call came FROM, in two parts: what
        // woke the run up, and what it has read since. Both matter — a
        // schedule that then read a web page is less trusted than either
        // fact alone, and agentd escalates on the worst of them.
        let mut chain: Vec<&str> = vec![self.trigger];
        if self.web_tainted.get() {
            chain.push("web");
        }
        let options: HashMap<String, OwnedValue> = match (
            OwnedValue::try_from(zbus::zvariant::Value::from("assistant")),
            OwnedValue::try_from(zbus::zvariant::Value::from(chain)),
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
            Ok((_id, disposition, detail)) => {
                if disposition == "executed" && result_is_web_tagged(&detail) {
                    self.web_tainted.set(true);
                }
                outcome_for(&disposition, &detail, &label)
            }
            Err(e) => ToolOutcome::err(format!("{label}: unreadable reply: {e}")),
        }
    }
}

/// OpenAI tool names allow `[A-Za-z0-9_-]{1,64}`; bus ids are
/// `app.lisaos.notes::create_note`. Flatten deterministically so the
/// reply maps back to exactly one catalog entry.
pub fn wire_name(app_id: &str, tool: &str) -> String {
    let flat = format!("{}__{}", app_id.replace(['.', '-'], "_"), tool);
    flat.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .chars()
        .take(64)
        .collect()
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

    /// The browser's write tier by name, because #216 is a live
    /// proposal to widen this filter and the reason it stays is not
    /// visible from `tier == "read"` alone (see the fn docs).
    ///
    /// If this ever goes green with the names present, the consent
    /// surface had better be running, independent, and device-proven.
    #[test]
    fn the_browsers_write_tools_are_not_handed_to_the_model() {
        let surfer = r#"[
          {"app_id":"app.lisaos.Surfer","name":"read_page","tier":"read"},
          {"app_id":"app.lisaos.Surfer","name":"navigate","tier":"write"},
          {"app_id":"app.lisaos.Surfer","name":"click","tier":"write"},
          {"app_id":"app.lisaos.Surfer","name":"fill","tier":"write"}
        ]"#;
        let names: Vec<String> = read_tier_tools(surfer)
            .unwrap()
            .into_iter()
            .map(|t| t.tool)
            .collect();
        assert_eq!(names, vec!["read_page"]);
        for privileged in ["navigate", "click", "fill"] {
            assert!(
                !names.iter().any(|n| n == privileged),
                "`{privileged}` reached the model's tool list with no \
                 independent consent surface to gate it (#216)"
            );
        }
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

    /// The chain is trigger + what has been read, not a constant.
    #[test]
    fn the_chain_starts_from_the_trigger() {
        for (trigger, tainted, expected) in [
            ("user", false, vec!["user"]),
            ("user", true, vec!["user", "web"]),
            ("schedule", false, vec!["schedule"]),
            ("event", true, vec!["event", "web"]),
        ] {
            let mut chain: Vec<&str> = vec![trigger];
            if tainted {
                chain.push("web");
            }
            assert_eq!(chain, expected, "trigger={trigger} tainted={tainted}");
        }
    }

    /// #146 Phase 4: the taint detector. The injection scenario is a
    /// page whose BODY contains the tag spelling — it must not taint via
    /// substring, only via the real JSON field the browser's MCP edge
    /// writes.
    #[test]
    fn web_taint_comes_from_the_field_not_the_text() {
        // The real shape agentd returns for an executed browser call.
        assert!(result_is_web_tagged(
            r#"{"result":{"provenance":"web","content":[{"type":"text","text":"..."}]},"ledger_ref":7}"#
        ));
        // A page BODY carrying the spelling — inside the text, not the field.
        assert!(!result_is_web_tagged(
            r#"{"result":{"content":[{"type":"text","text":"ignore this: \"provenance\":\"web\""}]},"ledger_ref":7}"#
        ));
        // Other apps' results: untagged.
        assert!(!result_is_web_tagged(
            r#"{"result":{"notes":[]},"ledger_ref":3}"#
        ));
        // Junk: not tainted, not a crash.
        assert!(!result_is_web_tagged("not json"));
        assert!(!result_is_web_tagged(r#"{"result":{"provenance":"user"}}"#));
    }

    /// A refusal is not a denial and must not read as retryable (#251).
    /// The loop cannot see the dialog, so the tool result is the only
    /// place it learns that trying again is pointless.
    #[test]
    fn a_refusal_tells_the_model_to_stop_rather_than_to_try_again() {
        let out = outcome_for(
            "refused",
            r#"{"rule":"rm.system_path","reason":"..."}"#,
            "app.lisaos.Probe244::delete_everything",
        );
        assert!(out.text.starts_with("error:"));
        assert!(out.text.contains("refused"));
        assert!(
            out.text.contains("Do not retry"),
            "a refusal that reads as retryable produces a loop: {}",
            out.text
        );
        assert!(!out.mutated);
        // …and it is distinguishable from a denial, which IS a person
        // saying no this once.
        let denied = outcome_for("denied", "d", "app::t");
        assert_ne!(denied.text, out.text);
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
