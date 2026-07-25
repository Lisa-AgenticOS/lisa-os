//! Skills — SKILL.md workflow files with progressive disclosure (the
//! flakerimi/harness "Skills" pillar). A skill is a markdown file with a
//! small frontmatter block:
//!
//! ```text
//! ---
//! name: deploy-demo
//! description: Build and deploy the demo site to the nuc box
//! tools: read_file, run_command
//! ---
//! 1. `just build` ...  (the full workflow, loaded only when used)
//! ```
//!
//! Every prompt carries only the one-line catalog (`name: description`);
//! [`Skill::body`] reads the full workflow from disk lazily, on use.
//! Frontmatter is parsed by hand — `key: value` lines only, no YAML dep.
//! `tools` is an optional comma-separated allowlist: absent means the
//! skill doesn't restrict tools; present (even empty) means only the
//! listed tools may be used while the skill drives.
//!
//! [`LoadReport::resolve`] routes a prompt to a skill with the same
//! deterministic token-overlap scoring the rest of the stack uses — no
//! model in the loop at this layer.

use std::path::{Path, PathBuf};

/// Mirror of registry.rs `tokens()` / agent.js `tokenize()`: lowercase,
/// split on non-ASCII-alphanumeric runs, drop empties.
pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// One discovered skill: frontmatter in memory, body on disk.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Optional tool allowlist (`tools:` frontmatter). `None` = the
    /// skill doesn't restrict tools; `Some(list)` = only those tools.
    pub tools: Option<Vec<String>>,
    path: PathBuf,
    /// Byte offset of the markdown body within `path`.
    body_offset: usize,
}

impl Skill {
    /// Discover skills under `dir` (recursively, `*.md`, sorted for
    /// determinism). Files that aren't valid skills are not errors —
    /// they're reported in [`LoadReport::skipped`] with the reason.
    pub fn load_dir(dir: impl AsRef<Path>) -> LoadReport {
        let mut files = Vec::new();
        collect_markdown(dir.as_ref(), &mut files);
        files.sort();

        let mut report = LoadReport::default();
        for path in files {
            match std::fs::read_to_string(&path) {
                Ok(text) => match parse_frontmatter(&text) {
                    Ok((fm, body_offset)) => report.skills.push(Skill {
                        name: fm.name,
                        description: fm.description,
                        tools: fm.tools,
                        path,
                        body_offset,
                    }),
                    Err(reason) => report.skipped.push(Skipped { path, reason }),
                },
                Err(e) => report.skipped.push(Skipped {
                    path,
                    reason: format!("unreadable: {e}"),
                }),
            }
        }
        report
    }

    /// The progressive-disclosure index line: `name: description`.
    pub fn catalog_line(&self) -> String {
        if self.description.is_empty() {
            self.name.clone()
        } else {
            format!("{}: {}", self.name, self.description)
        }
    }

    /// The full markdown workflow, read from disk on use.
    pub fn body(&self) -> Result<String, crate::Error> {
        let text = std::fs::read_to_string(&self.path)?;
        Ok(text
            .get(self.body_offset..)
            .unwrap_or("")
            .trim_start_matches(['\r', '\n'])
            .to_string())
    }

    /// Whether this skill permits `tool` (no allowlist = everything).
    pub fn allows_tool(&self, tool: &str) -> bool {
        match &self.tools {
            None => true,
            Some(list) => list.iter().any(|t| t == tool),
        }
    }

    /// Token-overlap relevance of this skill to [`tokenize`]d query
    /// tokens: a name-token hit weighs 3, a description hit 1 (the
    /// registry.rs / agent.js weights).
    pub fn score(&self, query_tokens: &[String]) -> i64 {
        let name_tokens = tokenize(&self.name);
        let desc_tokens = tokenize(&self.description);
        query_tokens
            .iter()
            .map(|q| {
                if name_tokens.contains(q) {
                    3
                } else if desc_tokens.contains(q) {
                    1
                } else {
                    0
                }
            })
            .sum()
    }
}

/// The outcome of a [`Skill::load_dir`] scan: what loaded, and what was
/// skipped (with why).
#[derive(Debug, Default)]
pub struct LoadReport {
    pub skills: Vec<Skill>,
    pub skipped: Vec<Skipped>,
}

/// A markdown file that didn't parse as a skill, and the reason.
#[derive(Debug)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

impl LoadReport {
    /// The catalog lines for every loaded skill, in discovery order —
    /// ready for [`crate::Turn::skill_catalog`].
    pub fn catalog(&self) -> Vec<String> {
        self.skills.iter().map(Skill::catalog_line).collect()
    }

    /// The skill this prompt is asking for, if any — deterministic
    /// token scoring, no model in the loop. A genuine skill-name token
    /// hit is required: registry.rs/agent.js approximate that with a
    /// score threshold, but several description hits can sum past any
    /// threshold, so this checks the name overlap itself. Ranking uses
    /// [`Skill::score`]; ties break by name, so resolution is stable
    /// across runs. `None` = no confident match; the prompt proceeds
    /// with the catalog only.
    pub fn resolve(&self, prompt: &str) -> Option<&Skill> {
        let query_tokens = tokenize(prompt);
        if query_tokens.is_empty() {
            return None;
        }
        self.skills
            .iter()
            .filter(|s| {
                let name_tokens = tokenize(&s.name);
                query_tokens.iter().any(|q| name_tokens.contains(q))
            })
            .map(|s| (s.score(&query_tokens), s))
            // max_by prefers later elements on ties, so compare name
            // reversed: the alphabetically-first name wins.
            .max_by(|(sa, a), (sb, b)| sa.cmp(sb).then_with(|| b.name.cmp(&a.name)))
            .map(|(_, s)| s)
    }
}

struct Frontmatter {
    name: String,
    description: String,
    tools: Option<Vec<String>>,
}

/// Parse `---`-delimited frontmatter, returning it plus the byte offset
/// where the markdown body starts. Only `name` is required.
fn parse_frontmatter(text: &str) -> Result<(Frontmatter, usize), String> {
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().ok_or("empty file")?;
    if first.trim_end() != "---" {
        return Err("no frontmatter".to_string());
    }
    let mut offset = first.len();
    let mut name = None;
    let mut description = String::new();
    let mut tools = None;
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed == "---" {
            let body_offset = offset + line.len();
            let Some(name) = name else {
                return Err("frontmatter missing `name`".to_string());
            };
            return Ok((
                Frontmatter {
                    name,
                    description,
                    tools,
                },
                body_offset,
            ));
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            match key.trim() {
                "name" if !value.trim().is_empty() => name = Some(value.trim().to_string()),
                "description" => description = value.trim().to_string(),
                // Presence of the key turns the allowlist on, even
                // empty: an explicit `tools:` restricts to nothing.
                "tools" => {
                    tools = Some(
                        value
                            .split(',')
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .map(str::to_string)
                            .collect(),
                    )
                }
                _ => {} // unknown keys are fine — forward compatibility
            }
        }
        offset += line.len();
    }
    Err("unterminated frontmatter".to_string())
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_dir_discovers_skills_and_reports_skips() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("deploy.md"),
            "---\nname: deploy-demo\ndescription: Deploy the demo site\n---\n\n1. just build\n2. just ship\n",
        )
        .unwrap();
        // Nested skill directories work too (the harness layout).
        let nested = dir.path().join("review");
        fs::create_dir(&nested).unwrap();
        fs::write(
            nested.join("SKILL.md"),
            "---\nname: code-review\n# a stray comment line in frontmatter is ignored\ndescription: Review a diff like a senior\n---\nLook for regressions first.\n",
        )
        .unwrap();
        // Not skills, each skipped with its reason:
        fs::write(dir.path().join("plain-notes.md"), "just some notes\n").unwrap();
        fs::write(dir.path().join("broken.md"), "---\nname: never-closed\n").unwrap();
        fs::write(
            dir.path().join("noname.md"),
            "---\ndescription: nope\n---\nbody\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("ignore.txt"),
            "not markdown, not even considered",
        )
        .unwrap();

        let report = Skill::load_dir(dir.path());
        assert_eq!(report.skills.len(), 2);
        assert_eq!(report.skipped.len(), 3);

        // Sorted by path: deploy.md < noname.md < plain-notes.md < review/SKILL.md
        assert_eq!(report.skills[0].name, "deploy-demo");
        assert_eq!(report.skills[1].name, "code-review");
        let reasons: Vec<&str> = report.skipped.iter().map(|s| s.reason.as_str()).collect();
        assert!(reasons.contains(&"no frontmatter"), "{reasons:?}");
        assert!(reasons.contains(&"unterminated frontmatter"), "{reasons:?}");
        assert!(
            reasons.contains(&"frontmatter missing `name`"),
            "{reasons:?}"
        );
    }

    #[test]
    fn catalog_line_and_lazy_body() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("deploy.md"),
            "---\nname: deploy-demo\ndescription: Deploy the demo site\n---\n\n1. just build\n2. just ship\n",
        )
        .unwrap();
        let report = Skill::load_dir(dir.path());
        assert_eq!(report.catalog(), vec!["deploy-demo: Deploy the demo site"]);

        let skill = &report.skills[0];
        // The body loads on demand, frontmatter excluded.
        let body = skill.body().unwrap();
        assert!(body.starts_with("1. just build"), "body: {body:?}");
        assert!(body.contains("2. just ship"));
        assert!(!body.contains("name:"));
    }

    #[test]
    fn tools_frontmatter_is_an_optional_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("open.md"),
            "---\nname: open-skill\ndescription: No restriction\n---\nbody\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("scoped.md"),
            "---\nname: scoped-skill\ntools: read_file, run_command\n---\nbody\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("sealed.md"),
            "---\nname: sealed-skill\ntools:\n---\nbody\n",
        )
        .unwrap();

        let report = Skill::load_dir(dir.path());
        assert_eq!(report.skills.len(), 3);
        let by_name = |n: &str| report.skills.iter().find(|s| s.name == n).unwrap();

        let open = by_name("open-skill");
        assert_eq!(open.tools, None);
        assert!(open.allows_tool("anything"));

        let scoped = by_name("scoped-skill");
        assert_eq!(
            scoped.tools.as_deref(),
            Some(&["read_file".to_string(), "run_command".to_string()][..])
        );
        assert!(scoped.allows_tool("read_file"));
        assert!(!scoped.allows_tool("delete_everything"));

        // An explicit empty `tools:` allows nothing — restriction is on.
        let sealed = by_name("sealed-skill");
        assert_eq!(sealed.tools.as_deref(), Some(&[][..]));
        assert!(!sealed.allows_tool("read_file"));
    }

    #[test]
    fn resolve_routes_by_name_tokens_deterministically() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("deploy.md"),
            "---\nname: deploy-demo\ndescription: Ship the demo site to the nuc box\n---\nbody\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("review.md"),
            "---\nname: code-review\ndescription: Review a diff like a senior\n---\nbody\n",
        )
        .unwrap();
        let report = Skill::load_dir(dir.path());

        // A name-token hit routes; scoring mirrors registry.rs/agent.js.
        let hit = report.resolve("deploy the new build").unwrap();
        assert_eq!(hit.name, "deploy-demo");
        assert_eq!(
            report.resolve("please review my diff").unwrap().name,
            "code-review"
        );

        // Description-only overlap stays below the threshold: catalog only.
        assert!(report.resolve("the nuc box is acting up").is_none());
        assert!(report.resolve("").is_none());
        assert!(report.resolve("!!!").is_none());

        // Ties break alphabetically by name — stable across runs.
        fs::write(
            dir.path().join("deploy2.md"),
            "---\nname: deploy-prod\ndescription: Ship to production\n---\nbody\n",
        )
        .unwrap();
        let report = Skill::load_dir(dir.path());
        assert_eq!(report.resolve("deploy it").unwrap().name, "deploy-demo");
        // An extra name token disambiguates.
        assert_eq!(report.resolve("deploy prod").unwrap().name, "deploy-prod");
    }

    #[test]
    fn missing_dir_is_an_empty_report_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let report = Skill::load_dir(dir.path().join("does-not-exist"));
        assert!(report.skills.is_empty());
        assert!(report.skipped.is_empty());
    }
}
