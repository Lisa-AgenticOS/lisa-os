//! Skills — how to do a thing, offered rather than injected, and the
//! one place that decides where they live.
//!
//! A skill is a markdown file with `name`/`description` frontmatter and a
//! workflow body ([`harness_core::Skill`]). The catalog — one
//! `name: description` line each — is small enough to sit in the system
//! prompt; the bodies are not, and pasting every skill into every
//! conversation is how a context window gets spent before the question
//! is read. So the model sees the list and fetches what it needs with
//! `read_skill`, a Read-tier operation over files we ship.
//!
//! **One search path, one loader (issue #245).** This module used to
//! exist twice: `cli/lisa/src/skills.rs` and `daemons/harnessd/src/skills.rs`,
//! the second claiming in a comment to "mirror `cli/lisa`'s resolution"
//! while spelling the runtime channel differently — so after a channel
//! update `lisa skills list` and the loop could see different sets. Both
//! spellings were wrong, which is the part worth remembering: the channel
//! is `/var/lib/lisa-apps/payloads/runtime/current` (`cli/lisa/src/apps.rs`,
//! `APPS_DIR`), and the two files had it under `/var/lib/lisa/apps/…`,
//! the pre-migration location. That is #239 exactly — a writer and a
//! reader spelling one path differently — and the fix is the same one:
//! a single authority both callers ask.

use crate::{ToolCall, ToolOutcome, ToolProvider, ToolSpec};
use harness_core::Skill;
use serde_json::json;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The tool that serves a skill body — and therefore the moment a skill
/// starts driving. The loop watches for this name to know which skill's
/// `tools:` allowlist is in force (issue #245); it is a constant here so
/// the loop and the provider cannot disagree about the spelling.
pub const READ_SKILL: &str = "read_skill";

/// Skills delivered by the runtime channel (issue #52): updated with
/// `lisa apps update runtime`, no reboot, no OS release.
///
/// Spelled the way `cli/lisa/src/apps.rs` spells it — `/var/lib/lisa-apps`,
/// beside the model store, because `/var/lib/lisa` is a `DynamicUser`
/// `StateDirectory` no ordinary user can traverse.
const CHANNEL_SKILLS_DIR: &str = "/var/lib/lisa-apps/payloads/runtime/current/skills";

/// Where releases up to the `/var/lib/lisa-apps` migration unpacked the
/// runtime channel (the `stale` list of the `runtime` channel in
/// `cli/lisa/src/apps.rs`). Read, never written: a device mid-upgrade
/// keeps resolving the skills it already has.
const STALE_CHANNEL_SKILLS_DIR: &str = "/var/lib/lisa/apps/payloads/runtime/current/skills";

/// The packaged skill set (installed by the lisa-cli package) — the floor
/// that exists on every system whether or not the channel has been synced.
const SYSTEM_SKILLS_DIR: &str = "/usr/share/lisa/skills";

/// Search path for skills, earlier directories winning on a name clash so
/// an override can shadow a packaged skill.
///
/// `over` is the `$LISA_SKILLS_DIR` value, which may hold several
/// `:`-separated directories (a dev checkout plus the packaged set, say).
pub fn skills_dirs_from(over: Option<OsString>, data_home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = over
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    dirs.push(data_home.join("lisa/skills"));
    dirs.push(PathBuf::from(CHANNEL_SKILLS_DIR));
    dirs.push(PathBuf::from(STALE_CHANNEL_SKILLS_DIR));
    dirs.push(PathBuf::from(SYSTEM_SKILLS_DIR));
    dirs
}

/// `$XDG_DATA_HOME`, or the spec default `~/.local/share`.
///
/// harnessd used to read `$HOME/.local/share` directly, so a user who had
/// moved `$XDG_DATA_HOME` got their skills in `lisa skills list` and not
/// in the loop.
pub fn user_data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|h| h.join(".local/share"))
        })
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The search path this machine resolves skills on.
pub fn skills_dirs() -> Vec<PathBuf> {
    skills_dirs_from(std::env::var_os("LISA_SKILLS_DIR"), &user_data_dir())
}

/// Every skill on `dirs`, earlier directories winning on a name clash.
pub fn load_from(dirs: &[PathBuf]) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        for skill in Skill::load_dir(dir).skills {
            if !out.iter().any(|s| s.name == skill.name) {
                out.push(skill);
            }
        }
    }
    out
}

/// Every skill on the search path.
pub fn load() -> Vec<Skill> {
    load_from(&skills_dirs())
}

/// The lines that go in the system prompt. Empty when there are no
/// skills — and then the prompt says nothing about them, rather than
/// advertising a tool that would return nothing.
pub fn catalog_lines(skills: &[Skill]) -> String {
    skills
        .iter()
        .map(|s| format!("- {}: {}", s.name, s.description))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `read_skill` — fetch one workflow body by name.
pub struct SkillTools {
    skills: Vec<Skill>,
}

impl SkillTools {
    pub fn new(skills: Vec<Skill>) -> SkillTools {
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
            READ_SKILL,
            "Read the full instructions for one of the skills listed in your \
             system prompt. Do this BEFORE starting a task a skill covers — the \
             list only gives you its name and one line. While a skill is loaded \
             you may only use the tools it declares.",
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

    /// The reader must look where the writer writes (#239, #245).
    ///
    /// `lisa apps` installs the runtime channel under `/var/lib/lisa-apps`
    /// — not `/var/lib/lisa/apps`, which is where both skill resolvers
    /// used to look, one of them for a directory (`apps/current/skills`)
    /// that no release has ever produced.
    #[test]
    fn the_channel_dir_is_the_one_the_runtime_channel_installs() {
        let dirs = skills_dirs_from(None, Path::new("/home/me/.local/share"));
        assert!(
            dirs.iter()
                .any(|d| d == Path::new("/var/lib/lisa-apps/payloads/runtime/current/skills")),
            "the runtime channel's own directory is not on the search path: {dirs:?}"
        );
        // The pre-migration location stays readable so a device that has
        // not taken the migration release still finds its skills.
        assert!(
            dirs.iter()
                .any(|d| d == Path::new("/var/lib/lisa/apps/payloads/runtime/current/skills")),
            "the pre-migration channel dir was dropped: {dirs:?}"
        );
        // And the spelling nothing ever wrote is gone for good.
        assert!(
            !dirs
                .iter()
                .any(|d| d == Path::new("/var/lib/lisa/apps/current/skills")),
            "a path no release ever installed is still searched: {dirs:?}"
        );
    }

    /// Order is the whole point: an override shadows the channel, the
    /// channel shadows the packaged floor.
    #[test]
    fn earlier_directories_win() {
        let dirs = skills_dirs_from(
            Some(OsString::from("/opt/a:/opt/b")),
            Path::new("/home/me/.local/share"),
        );
        assert_eq!(dirs[0], Path::new("/opt/a"));
        assert_eq!(dirs[1], Path::new("/opt/b"));
        assert_eq!(dirs[2], Path::new("/home/me/.local/share/lisa/skills"));
        assert_eq!(dirs.last().unwrap(), Path::new(SYSTEM_SKILLS_DIR));

        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(
            first.join("s.md"),
            "---\nname: dup\ndescription: the override\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            second.join("s.md"),
            "---\nname: dup\ndescription: the packaged one\n---\nbody\n",
        )
        .unwrap();
        let loaded = load_from(&[first, second]);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].description, "the override");
    }

    #[test]
    fn an_empty_catalog_offers_no_tool() {
        // Advertising read_skill with nothing to read wastes a turn on a
        // tool that can only fail.
        let t = SkillTools::new(Vec::new());
        assert!(t.specs().is_empty());
        assert!(t.is_empty());
    }

    #[test]
    fn the_tool_the_loop_watches_for_is_the_one_advertised() {
        // The loop keys skill activation off this name; if the provider
        // renamed its tool the allowlist would silently stop engaging.
        let t = SkillTools::new(vec![Skill::declared("demo", None)]);
        assert_eq!(t.specs()[0].name, READ_SKILL);
    }
}
