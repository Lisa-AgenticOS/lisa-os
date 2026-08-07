//! `lisa skills` — the SKILL.md workflows Lisa loads on demand (ADR-0025,
//! Skills pillar). A skill is a markdown file with `name`/`description`
//! frontmatter; only the one-line description is meant to stay resident in
//! a prompt, and the body is read when it is actually needed. That is what
//! `list` and `show` expose here.
//!
//! **Resolution is not decided here** (issue #245). It lives in
//! `forge_harness::skills`, which is what the agent loop uses too, so
//! `lisa skills list` and the loop cannot answer differently. They did:
//! this file spelled the runtime channel
//! `/var/lib/lisa/apps/payloads/runtime/current/skills` and harnessd
//! spelled it `/var/lib/lisa/apps/current/skills`, and `lisa apps`
//! installs neither — it installs
//! `/var/lib/lisa-apps/payloads/runtime/current/skills`.

use anyhow::bail;
use forge_harness::skills::{load, skills_dirs};

/// `lisa skills list`: the catalog — one line per skill, the shape a
/// prompt carries.
pub fn list() -> anyhow::Result<()> {
    let skills = load();
    if skills.is_empty() {
        println!(
            "no skills found — looked in {}",
            skills_dirs()
                .iter()
                .map(|d| d.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Ok(());
    }
    for skill in &skills {
        println!("{}", skill.catalog_line());
    }
    Ok(())
}

/// `lisa skills show <name>`: the full workflow, read from disk on use.
pub fn show(name: &str) -> anyhow::Result<()> {
    let skills = load();
    let Some(skill) = skills.iter().find(|s| s.name == name) else {
        bail!("no skill named {name} — `lisa skills list` shows what is installed");
    };
    print!("{}", skill.body()?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `lisa skills list` and the agent loop must look in the same
    /// places (#245). The ordering itself is asserted where it is
    /// decided, in `forge_harness::skills` — a second copy of those
    /// assertions here would be a third opinion about one path.
    #[test]
    fn resolution_is_the_shared_one() {
        assert_eq!(skills_dirs(), forge_harness::skills::skills_dirs());
    }

    #[test]
    fn the_repos_own_skills_parse_and_expose_a_description() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../skills")
            .canonicalize()
            .expect("skills/ lives at the repo root");
        let report = harness_core::Skill::load_dir(&repo);
        assert!(
            report.skipped.is_empty(),
            "every shipped skill must parse: {:?}",
            report.skipped
        );
        let build = report
            .skills
            .iter()
            .find(|s| s.name == "build-lisa-app")
            .expect("the app-building skill ships");
        assert!(!build.description.is_empty());
        // Progressive disclosure: the catalog line is the cheap part, the
        // body is the expensive part and only loads on demand.
        assert!(build.catalog_line().len() < 200);
        let body = build.body().expect("body reads");
        // ADR-0047 (amended 2026-08-07): GJS + GTK4/Adwaita is the ONE
        // toolkit — the Flutter lane is removed, not parked. This
        // assertion is the reason the skill cannot quietly drift back —
        // it used to open "Apps on Lisa are Flutter apps".
        assert!(
            body.contains("GJS + GTK4/Adwaita is the one toolkit"),
            "the toolkit is the point"
        );
        assert!(body.contains("/usr/share/lisa/manifests/"), "trap #241");
        // Every tool the allowlist names must be one the loop can
        // actually dispatch: `list_files` was named for years and does
        // not exist, so the skill forbade the `list_dir` it meant to use.
        let known = [
            "read_file",
            "list_dir",
            "grep",
            "write_file",
            "edit_file",
            "run_command",
            "run_tests",
            "run_shell",
            "read_skill",
        ];
        for tool in build.tools.as_deref().expect("the skill scopes itself") {
            assert!(known.contains(&tool.as_str()), "no such tool: {tool}");
        }
    }
}
