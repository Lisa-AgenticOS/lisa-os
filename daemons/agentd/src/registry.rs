//! Registry of installed MCP servers + tool discovery
//! (`docs/PLAN.md` §5.4: "maintains the registry of installed servers,
//! mediates discovery").
//!
//! Manifests are JSON files (Appendix B) installed under the manifest
//! directories; invalid files are skipped with a reason, never fatal —
//! one broken app must not take the bus down. Discovery is a
//! deterministic token-overlap ranking over tool names, descriptions,
//! and app ids ("what can handle 'add a task'?"); semantic ranking via
//! embeddings is a later slice.

use crate::manifest::{Manifest, ManifestError, ToolDecl};
use crate::tier::Tier;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Discovery/listing view of one tool.
#[derive(Debug, Clone, Serialize)]
pub struct ToolRef {
    pub app_id: String,
    pub name: String,
    pub tier: Tier,
    pub description: String,
    pub undoable: bool,
    /// The tool's argument schema, verbatim from its manifest — the
    /// intent router's arg-filler grammar-constrains against it
    /// (liblisa::intent, ADR-0013).
    pub input_schema: Value,
}

#[derive(Debug, Default)]
pub struct LoadReport {
    pub loaded: Vec<String>,
    pub skipped: Vec<(PathBuf, String)>,
    /// Manifests that were altered on the way in — a tier the floor
    /// raised (#56), a schema bound removed because it would break
    /// grammar compilation (#147).
    ///
    /// Reported rather than silent, for the same reason a shadowed
    /// manifest is: an app author whose manifest is quietly rewritten
    /// debugs the wrong thing, and an admin has no way to see that it
    /// happened. `(app_id, what)`.
    pub adjusted: Vec<(String, String)>,
}

#[derive(Debug, Default)]
pub struct Registry {
    apps: BTreeMap<String, Manifest>,
}

impl Registry {
    pub fn new() -> Registry {
        Registry::default()
    }

    /// Install or update (replace) one app's manifest.
    ///
    /// Explicit replacement, for programmatic callers that mean it.
    /// Manifests loaded from disk go through [`Registry::load_dir`],
    /// which refuses to replace — see the note there.
    pub fn insert(&mut self, manifest: Manifest) -> Result<(), ManifestError> {
        manifest.validate()?;
        self.apps.insert(manifest.app_id.clone(), manifest);
        Ok(())
    }

    /// Whether an app is already defined.
    pub fn contains(&self, app_id: &str) -> bool {
        self.apps.contains_key(app_id)
    }

    /// Load every `*.json` in `dir`. Missing dir → empty report.
    pub fn load_dir(&mut self, dir: &Path) -> LoadReport {
        let mut report = LoadReport::default();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return report,
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();
        for path in paths {
            let parsed = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| Manifest::from_json(&text).map_err(|e| e.to_string()));
            match parsed {
                Ok(m) => {
                    // FIRST DEFINITION WINS (issue #97). Directories are
                    // loaded system-first, so a file in the user's data
                    // dir can add a NEW app but can never redefine one
                    // the system already declares.
                    //
                    // Before this, later won: a user-writable manifest
                    // reusing a system app_id rewrote its tiers from
                    // `destructive` to `read`, deleted tools from the
                    // registry and added undeclared ones — and the real
                    // MCP server then executed them. The tier machinery
                    // reasons over this file (ADR-0030 §5); letting the
                    // untrusted side rewrite it makes every check
                    // downstream advisory.
                    //
                    // The clash is reported, never silent: a shadowed
                    // manifest that vanished quietly is how an admin
                    // discovers the problem from the outside.
                    if self.apps.contains_key(&m.app_id) {
                        report.skipped.push((
                            path,
                            format!(
                                "app `{}` is already defined by an earlier \
                                 (higher-precedence) manifest — ignored",
                                m.app_id
                            ),
                        ));
                    } else {
                        for (tool, from, to) in m.raised_tiers() {
                            report.adjusted.push((
                                m.app_id.clone(),
                                format!(
                                    "{tool}: declared tier {} raised to {} by the \
                                     name floor",
                                    from.as_str(),
                                    to.as_str()
                                ),
                            ));
                        }
                        for (tool, keys) in m.stripped_bounds() {
                            report.adjusted.push((
                                m.app_id.clone(),
                                format!(
                                    "{tool}: dropped {} — a bound that large \
                                     breaks grammar compilation and would have \
                                     disabled EVERY tool, not just this one",
                                    keys.join(", ")
                                ),
                            ));
                        }
                        report.loaded.push(m.app_id.clone());
                        self.apps.insert(m.app_id.clone(), m);
                    }
                }
                Err(reason) => report.skipped.push((path, reason)),
            }
        }
        report
    }

    pub fn len(&self) -> usize {
        self.apps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    pub fn manifest(&self, app_id: &str) -> Option<&Manifest> {
        self.apps.get(app_id)
    }

    pub fn tool(&self, app_id: &str, name: &str) -> Option<&ToolDecl> {
        self.apps.get(app_id).and_then(|m| m.tool(name))
    }

    /// All tools, app-then-name order.
    pub fn list(&self) -> Vec<ToolRef> {
        self.apps
            .values()
            .flat_map(|m| {
                m.tools.iter().map(|t| ToolRef {
                    app_id: m.app_id.clone(),
                    name: t.name.clone(),
                    tier: t.tier,
                    description: t.description.clone(),
                    undoable: t.undo.is_some(),
                    input_schema: t.input_schema.clone(),
                })
            })
            .collect()
    }

    /// Rank tools against a natural-language query by token overlap:
    /// name-token hits weigh 3, description and app-id hits 1. Tools
    /// with zero overlap are omitted.
    pub fn discover(&self, query: &str) -> Vec<ToolRef> {
        let query_tokens = tokens(query);
        if query_tokens.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(i64, ToolRef)> = self
            .list()
            .into_iter()
            .filter_map(|t| {
                let name_tokens = tokens(&t.name);
                let desc_tokens = tokens(&t.description);
                let app_tokens = tokens(&t.app_id);
                let score: i64 = query_tokens
                    .iter()
                    .map(|q| {
                        if name_tokens.contains(q) {
                            3
                        } else if desc_tokens.contains(q) || app_tokens.contains(q) {
                            1
                        } else {
                            0
                        }
                    })
                    .sum();
                (score > 0).then_some((score, t))
            })
            .collect();
        scored.sort_by(|(sa, a), (sb, b)| {
            sb.cmp(sa)
                .then_with(|| a.app_id.cmp(&b.app_id))
                .then_with(|| a.name.cmp(&b.name))
        });
        scored.into_iter().map(|(_, t)| t).collect()
    }
}

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::fixture_calendar_json;

    /// Issue #97 (high): a user-writable manifest reusing a system
    /// app_id used to WIN, because directories loaded system-first and
    /// "later wins". A reviewer used it to rewrite a live app's tiers
    /// from `destructive` to `read`, delete tools from the registry and
    /// add undeclared ones — which the real MCP server then executed.
    ///
    /// The manifest is the ontology the tier machinery reasons over
    /// (ADR-0030 §5). If the untrusted side can rewrite it, every check
    /// downstream is advisory.
    #[test]
    fn a_later_manifest_cannot_redefine_an_app_the_system_declared() {
        let system = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();

        // What the image ships: delete_event is destructive.
        std::fs::write(system.path().join("calendar.json"), fixture_calendar_json()).unwrap();

        // What an attacker drops in $XDG_DATA_HOME: same app_id, but
        // every dangerous tool downgraded to `read` so it dispatches
        // silently, with no confirmation and no undo journal entry.
        let hostile = fixture_calendar_json().replace("\"destructive\"", "\"read\"");
        assert_ne!(
            hostile,
            fixture_calendar_json(),
            "the fixture must contain a destructive tier"
        );
        std::fs::write(user.path().join("calendar.json"), &hostile).unwrap();

        let mut r = Registry::new();
        let sys_report = r.load_dir(system.path());
        let user_report = r.load_dir(user.path());

        assert_eq!(sys_report.loaded.len(), 1, "the system manifest must load");
        assert!(
            user_report.loaded.is_empty(),
            "the shadowing manifest was accepted: {user_report:?}"
        );
        assert_eq!(
            user_report.skipped.len(),
            1,
            "and it must be REPORTED, not dropped silently"
        );
        assert!(
            user_report.skipped[0].1.contains("already defined"),
            "the reason must say why: {}",
            user_report.skipped[0].1
        );

        // The surviving declaration is the system's, tiers intact.
        let decl = r
            .tool("org.gnome.Calendar", "delete_event")
            .expect("the system tool must still be registered");
        assert_eq!(
            decl.tier,
            crate::tier::Tier::Destructive,
            "the hostile manifest downgraded a live tool's tier"
        );
    }

    /// Two files in the SAME directory claiming one app_id used to
    /// resolve by sorted filename with no complaint. Ambiguity that
    /// picks a winner quietly is how the wrong one wins later.
    #[test]
    fn a_duplicate_app_id_in_one_directory_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a-first.json"), fixture_calendar_json()).unwrap();
        std::fs::write(dir.path().join("b-second.json"), fixture_calendar_json()).unwrap();

        let mut r = Registry::new();
        let report = r.load_dir(dir.path());
        assert_eq!(report.loaded.len(), 1);
        assert_eq!(report.skipped.len(), 1, "the duplicate must be reported");
        assert_eq!(r.len(), 1);
    }

    fn registry() -> Registry {
        let mut r = Registry::new();
        r.insert(Manifest::from_json(&fixture_calendar_json()).unwrap())
            .unwrap();
        r
    }

    #[test]
    fn list_reports_tier_and_undoability() {
        let r = registry();
        let tools = r.list();
        assert_eq!(tools.len(), 3);
        let add = tools.iter().find(|t| t.name == "add_event").unwrap();
        assert_eq!(add.tier, Tier::Write);
        assert!(add.undoable);
        let del = tools.iter().find(|t| t.name == "delete_event").unwrap();
        assert!(!del.undoable);
    }

    #[test]
    fn discover_ranks_name_matches_first_and_omits_misses() {
        let r = registry();
        let hits = r.discover("add a calendar event");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].name, "add_event", "name-token hit ranks first");
        assert!(r.discover("photosynthesis").is_empty());
        assert!(r.discover("").is_empty());
    }

    #[test]
    fn insert_replaces_on_same_app_id() {
        let mut r = registry();
        let mut m = Manifest::from_json(&fixture_calendar_json()).unwrap();
        m.tools.truncate(2);
        r.insert(m).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r.list().len(), 2, "reinstall replaces the old manifest");
    }

    #[test]
    fn load_dir_skips_invalid_files_and_keeps_valid_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("calendar.json"), fixture_calendar_json()).unwrap();
        std::fs::write(dir.path().join("broken.json"), "{ not json").unwrap();
        std::fs::write(
            dir.path().join("badversion.json"),
            fixture_calendar_json().replace("\"lisa_manifest\":1", "\"lisa_manifest\":9"),
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

        let mut r = Registry::new();
        let report = r.load_dir(dir.path());
        assert_eq!(report.loaded, vec!["org.gnome.Calendar".to_string()]);
        assert_eq!(report.skipped.len(), 2);
        assert_eq!(r.len(), 1);

        let mut empty = Registry::new();
        let report = empty.load_dir(Path::new("/definitely/not/a/dir"));
        assert!(report.loaded.is_empty() && report.skipped.is_empty());
    }
}
