//! Skills, as this daemon sees them — a thin view onto the one
//! resolver.
//!
//! The search path, the loader, the catalog lines and the `read_skill`
//! tool all live in `forge_harness::skills` now (issue #245). They used
//! to live here *as well*, and the copy here spelled the runtime channel
//! `/var/lib/lisa/apps/current/skills` while `cli/lisa` spelled it
//! `/var/lib/lisa/apps/payloads/runtime/current/skills` — a comment
//! above the copy claimed it "mirrors `cli/lisa`'s resolution". Neither
//! spelling was what `lisa apps` installs, so after a runtime-channel
//! update `lisa skills list` and the loop could see different sets. That
//! is #239 with different nouns, and the fix is the same: one authority,
//! two callers.
//!
//! The guard is structural rather than a test: there is one definition
//! and this is a re-export of it, so the two surfaces cannot spell the
//! search path differently even by accident. `cli/lisa/src/skills.rs`
//! asserts its own resolution IS the shared one, which is the thing a
//! future local copy here would have to break.
pub use forge_harness::skills::{SkillTools, catalog_lines, load};

#[cfg(test)]
mod tests {
    use super::*;
    use forge_harness::ToolProvider;

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
