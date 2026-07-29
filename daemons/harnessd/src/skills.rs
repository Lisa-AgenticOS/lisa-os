//! Skills: how to do a thing, offered rather than injected.
//!
//! A skill is a markdown file with `name`/`description` frontmatter and a
//! workflow body (`harness_core::Skill`). The catalog — one
//! `name: description` line each — is small enough to sit in the system
//! prompt; the bodies are not, and pasting every skill into every
//! conversation is how a context window gets spent before the question
//! is read.
//!
//! So the model sees the list and fetches what it needs with
//! `read_skill`. That is a Read-tier operation over files we ship, and it
//! is the reason a skill can be long and specific instead of a summary.

use forge_harness::{ToolCall, ToolOutcome, ToolProvider, ToolSpec};
use serde_json::json;
use std::path::PathBuf;

const CHANNEL_SKILLS_DIR: &str = "/var/lib/lisa/apps/current/skills";
const SYSTEM_SKILLS_DIR: &str = "/usr/share/lisa/skills";

/// Search path, earlier winning on a name clash so an override can
/// shadow a packaged skill. Mirrors `cli/lisa`'s resolution.
pub fn skills_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("LISA_SKILLS_DIR")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/lisa/skills"));
    }
    dirs.push(PathBuf::from(CHANNEL_SKILLS_DIR));
    dirs.push(PathBuf::from(SYSTEM_SKILLS_DIR));
    dirs
}

pub fn load() -> Vec<harness_core::Skill> {
    let mut out: Vec<harness_core::Skill> = Vec::new();
    for dir in skills_dirs() {
        if !dir.is_dir() {
            continue;
        }
        for s in harness_core::Skill::load_dir(&dir).skills {
            if !out.iter().any(|k| k.name == s.name) {
                out.push(s);
            }
        }
    }
    out
}

/// The lines that go in the system prompt. Empty when there are no
/// skills — and then the prompt says nothing about them, rather than
/// advertising a tool that would return nothing.
pub fn catalog_lines(skills: &[harness_core::Skill]) -> String {
    skills
        .iter()
        .map(|s| format!("- {}: {}", s.name, s.description))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `read_skill` — fetch one workflow body by name.
pub struct SkillTools {
    skills: Vec<harness_core::Skill>,
}

impl SkillTools {
    pub fn new(skills: Vec<harness_core::Skill>) -> SkillTools {
        SkillTools { skills }
    }
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

impl ToolProvider for SkillTools {
    fn specs(&self) -> Vec<ToolSpec> {
        if self.skills.is_empty() {
            return Vec::new();
        }
        vec![ToolSpec::new(
            "read_skill",
            "Read the full instructions for one of the skills listed in your \
             system prompt. Do this BEFORE starting a task a skill covers — the \
             list only gives you its name and one line.",
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The skill's name."},
                },
                "required": ["name"],
            }),
        )]
    }

    fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let Some(name) = call.args.get("name").and_then(|v| v.as_str()) else {
            return ToolOutcome::err("read_skill needs a `name`");
        };
        match self.skills.iter().find(|s| s.name == name) {
            // The names it may ask for are the ones we listed, so a miss
            // is worth naming them again rather than a bare not-found.
            None => ToolOutcome::err(format!(
                "no skill called {name:?}. Available: {}",
                self.skills
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Some(skill) => match skill.body() {
                Ok(body) => ToolOutcome::ok(body, false),
                Err(e) => ToolOutcome::err(format!("reading skill {name:?}: {e}")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_catalog_offers_no_tool() {
        // Advertising read_skill with nothing to read wastes a turn on a
        // tool that can only fail.
        let t = SkillTools::new(Vec::new());
        assert!(t.specs().is_empty());
        assert!(t.is_empty());
    }

    #[test]
    fn the_catalog_is_one_line_per_skill() {
        let skills = load();
        let lines = catalog_lines(&skills);
        assert_eq!(
            lines.lines().count(),
            skills.len(),
            "a multi-line description would break the prompt's list"
        );
    }
}
