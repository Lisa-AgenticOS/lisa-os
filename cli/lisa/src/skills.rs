//! `lisa skills` — the SKILL.md workflows Lisa loads on demand (ADR-0025,
//! Skills pillar). A skill is a markdown file with `name`/`description`
//! frontmatter; only the one-line description is meant to stay resident in
//! a prompt, and the body is read when it is actually needed. That is what
//! `list` and `show` expose here, and what the harness loop's `load_skill`
//! tool will call when phase 4 lands — same files, same resolution order.
//!
//! Resolution (first directory that defines a name wins):
//! `$LISA_SKILLS_DIR` → `$XDG_DATA_HOME/lisa/skills` → the runtime channel
//! (`/var/lib/lisa/apps/payloads/runtime/current/skills`, issue #52) →
//! `/usr/share/lisa/skills`. The channel sits ahead of the packaged set so a
//! skill can be taught without an OS release; the packaged set is the floor.

use anyhow::bail;
use std::ffi::OsString;
use std::path::PathBuf;

/// The packaged skill set (installed by the lisa-cli package) — the floor
/// that exists on every system whether or not the channel has been synced.
const SYSTEM_SKILLS_DIR: &str = "/usr/share/lisa/skills";

/// Skills delivered by the runtime channel (issue #52): updated with
/// `lisa apps update runtime`, no reboot, no OS release.
const CHANNEL_SKILLS_DIR: &str = "/var/lib/lisa/apps/payloads/runtime/current/skills";

/// Search path for skills; `over` is the `$LISA_SKILLS_DIR` value, which
/// may hold several `:`-separated directories (a dev checkout plus the
/// packaged set, say).
pub fn skills_dirs_from(over: Option<OsString>, data_home: &std::path::Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = over
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    dirs.push(data_home.join("lisa/skills"));
    dirs.push(PathBuf::from(CHANNEL_SKILLS_DIR));
    dirs.push(PathBuf::from(SYSTEM_SKILLS_DIR));
    dirs
}

fn skills_dirs() -> Vec<PathBuf> {
    skills_dirs_from(std::env::var_os("LISA_SKILLS_DIR"), &crate::user_data_dir())
}

/// Every skill on the search path, earlier directories winning on a name
/// clash (an override must be able to shadow the packaged skill).
fn load() -> Vec<harness_core::Skill> {
    let mut out: Vec<harness_core::Skill> = Vec::new();
    for dir in skills_dirs() {
        if !dir.is_dir() {
            continue;
        }
        for skill in harness_core::Skill::load_dir(&dir).skills {
            if !out.iter().any(|s| s.name == skill.name) {
                out.push(skill);
            }
        }
    }
    out
}

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

    #[test]
    fn override_dirs_come_first_and_the_packaged_set_last() {
        let dirs = skills_dirs_from(
            Some(OsString::from("/a:/b")),
            std::path::Path::new("/home/u/.local/share"),
        );
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/home/u/.local/share/lisa/skills"),
                // The runtime channel sits between the user's own skills
                // and the packaged floor (issue #52): a skill can be
                // taught without an OS release, but never shadows an
                // override the user placed deliberately.
                PathBuf::from(CHANNEL_SKILLS_DIR),
                PathBuf::from(SYSTEM_SKILLS_DIR),
            ]
        );
        // No override: the user's own skills still come first, and the
        // packaged set is always last so it can never shadow anything.
        let dirs = skills_dirs_from(None, std::path::Path::new("/home/u/.local/share"));
        assert_eq!(dirs.len(), 3);
        assert!(dirs[0].ends_with("lisa/skills"));
        assert_eq!(dirs[dirs.len() - 1], PathBuf::from(SYSTEM_SKILLS_DIR));
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
        // ADR-0047: GJS + GTK4/Adwaita is the default and Flutter is
        // parked. This assertion is the reason the skill cannot quietly
        // drift back — it used to open "Apps on Lisa are Flutter apps".
        assert!(
            body.contains("GJS + GTK4/Adwaita is the default toolkit"),
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
