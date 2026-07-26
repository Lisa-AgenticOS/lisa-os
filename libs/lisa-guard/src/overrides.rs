//! Human-set relaxations of the guard (ADR-0030).
//!
//! ADR-0029 made [`Verdict::Deny`] absolute, and that was a category
//! error. The governing principle is *probabilistic reasoning inside,
//! logical guardrails outside* — and **the human is outside**, on the
//! same side of the boundary as the guard, not the thing it is pointed
//! at. A guardrail belongs between the model and the machine; one placed
//! between the owner and their own machine is aimed at the wrong side.
//!
//! So a `Deny` can be relaxed — under one invariant that keeps the
//! principle intact:
//!
//! > **The boundary must not be reachable from inside.**
//!
//! This mechanism satisfies it by construction. Relaxations live in a
//! file the person edits out-of-band (or via `lisa guard allow`), read
//! ambiently at startup. There is no tool, no argument, no flag and no
//! dialog that reaches it, so nothing the model emits — and nothing a
//! retrieved document talks it into — can widen its own permissions. A
//! confirmation the model can re-trigger until you click yes would fail
//! that test; this does not.
//!
//! A relaxed rule becomes [`Verdict::Confirm`], never [`Verdict::Allow`]:
//! you asked for the block to be lifted, not for the action to go
//! unmentioned.

use crate::Verdict;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The set of rule ids whose `Deny` this machine's owner has relaxed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    relaxed: BTreeSet<String>,
}

impl Overrides {
    pub fn new() -> Self {
        Self::default()
    }

    /// One rule id per line. `#` starts a comment; blanks are ignored.
    pub fn parse(text: &str) -> Self {
        let relaxed = text
            .lines()
            .map(|line| line.split('#').next().unwrap_or("").trim())
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
        Self { relaxed }
    }

    /// Load from `path`. A missing file is not an error — it is the
    /// default posture, which is "nothing relaxed".
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// The file's canonical form, with a header explaining itself to
    /// whoever opens it next.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "# Guard rules this machine's owner has relaxed (ADR-0030).\n\
             #\n\
             # A rule listed here stops being a hard refusal and becomes a\n\
             # warning you can act on. It is never silenced entirely.\n\
             #\n\
             # Nothing the agent runs can read or write this file, which is\n\
             # what makes it safe to have: you can widen your own limits,\n\
             # and a web page the model happened to read cannot.\n\
             #\n\
             # Manage with `lisa guard allow|forbid|list`.\n\n",
        );
        for rule in &self.relaxed {
            out.push_str(rule);
            out.push('\n');
        }
        out
    }

    /// Returns whether this changed anything.
    pub fn allow(&mut self, rule: &str) -> bool {
        self.relaxed.insert(rule.trim().to_string())
    }

    /// Returns whether this changed anything.
    pub fn forbid(&mut self, rule: &str) -> bool {
        self.relaxed.remove(rule.trim())
    }

    pub fn is_relaxed(&self, rule: &str) -> bool {
        self.relaxed.contains(rule)
    }

    pub fn is_empty(&self) -> bool {
        self.relaxed.is_empty()
    }

    pub fn rules(&self) -> impl Iterator<Item = &str> {
        self.relaxed.iter().map(String::as_str)
    }

    /// Downgrade a relaxed `Deny` to `Confirm`. Everything else passes
    /// through untouched — this can only ever make the guard *less*
    /// strict for rules the owner named, and never turns a refusal into
    /// silence.
    pub fn relax(&self, verdict: Verdict) -> Verdict {
        match verdict {
            Verdict::Deny { rule, reason } if self.relaxed.contains(rule) => Verdict::Confirm {
                rule,
                reason: format!(
                    "{reason} — normally refused, relaxed by `lisa guard allow {rule}`"
                ),
            },
            other => other,
        }
    }
}

/// Where relaxations live: `$XDG_CONFIG_HOME/lisa/guard-allow`, falling
/// back to `~/.config/lisa/guard-allow`.
///
/// Deliberately in the user's config, not in the project and not
/// anywhere an agent has a tool for: the forge jail confines writes to
/// the project directory, and no guard-facing surface can write here.
pub fn overrides_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("lisa").join("guard-allow"))
}

/// The relaxations in effect for this user, or none if unreadable.
pub fn active() -> Overrides {
    overrides_path()
        .map(|p| Overrides::load(&p))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_ignores_comments_and_blanks() {
        let o = Overrides::parse("# header\n\nescalate.privilege\n  net.egress  # inline\n\n");
        assert!(o.is_relaxed("escalate.privilege"));
        assert!(o.is_relaxed("net.egress"));
        assert_eq!(o.rules().count(), 2);
    }

    #[test]
    fn a_relaxed_deny_becomes_a_warning_not_silence() {
        let mut o = Overrides::new();
        o.allow("escalate.privilege");

        let relaxed = o.relax(Verdict::deny("escalate.privilege", "runs as root"));
        assert!(relaxed.is_overridable(), "should ask, not refuse");
        assert!(!relaxed.is_allowed(), "must never become silent");
        assert!(relaxed.reason().unwrap().contains("lisa guard allow"));
    }

    #[test]
    fn unlisted_rules_are_untouched() {
        let mut o = Overrides::new();
        o.allow("net.egress");
        assert!(
            o.relax(Verdict::deny("rm.system_path", "deletes /"))
                .is_denied()
        );
        assert_eq!(o.relax(Verdict::Allow), Verdict::Allow);
        let confirm = Verdict::confirm("git.destructive", "loses work");
        assert_eq!(o.relax(confirm.clone()), confirm);
    }

    #[test]
    fn round_trips_through_the_file_format() {
        let mut o = Overrides::new();
        o.allow("escalate.privilege");
        o.allow("net.egress");
        assert_eq!(Overrides::parse(&o.render()), o);

        assert!(o.forbid("net.egress"));
        assert!(!o.forbid("net.egress"), "forbidding twice changes nothing");
        assert!(!o.is_relaxed("net.egress"));
    }

    #[test]
    fn a_missing_file_relaxes_nothing() {
        assert!(Overrides::load(Path::new("/nonexistent/lisa/guard-allow")).is_empty());
    }
}
