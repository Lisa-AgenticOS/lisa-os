//! Confirmation tiers + provenance escalation (`docs/PLAN.md` §5.4,
//! §5.10, Appendix C; CLAUDE.md rule 6).
//!
//! Policy, enforced at the bus, not by app goodwill:
//! - *read* → silent, ledgered;
//! - *write* → inline confirmation chip;
//! - *destructive* (incl. financial / external-send) → explicit modal
//!   with a typed diff of what will happen.
//!
//! Escalation: when the trigger chain includes **any** untrusted
//! provenance, the call is treated one tier up. Only `user` is trusted;
//! `file`/`mail`/`screen`/`web`, app-originated content, unknown tags,
//! and an *empty* chain (unknown origin) all escalate — provenance is
//! load-bearing, so absence of provenance fails closed.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Declared sensitivity of a tool (Appendix B `"tier"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Read,
    Write,
    Destructive,
}

impl Tier {
    /// Privileged tiers are the ones the M5 acceptance gate counts:
    /// anything that changes state outside the conversation.
    pub fn is_privileged(self) -> bool {
        self != Tier::Read
    }

    /// One tier up; `Destructive` is already the ceiling.
    pub fn escalated(self) -> Tier {
        match self {
            Tier::Read => Tier::Write,
            Tier::Write | Tier::Destructive => Tier::Destructive,
        }
    }

    /// The lowest tier a tool with this name could honestly declare
    /// (issue #56). `None` when the name implies nothing.
    ///
    /// A manifest states its own tiers and nothing checked them, so an
    /// app could declare `delete_everything` as `read` and get SILENT
    /// execution — no confirmation, no undo journal entry, because undo
    /// is gated on `is_privileged`. The manifest is the ontology the
    /// whole tier machinery reasons over (ADR-0030 §5); a lie in it is
    /// not a mistake the rest of the system can recover from.
    ///
    /// Deliberately name-based and deliberately coarse. This cannot
    /// detect a *dishonest* tool — `harmless_helper` that deletes your
    /// mail is undetectable from a manifest — and it is not trying to.
    /// It catches the case where the name says one thing and the tier
    /// says another, which is the difference between a lie and a lie
    /// that also bothered to hide.
    pub fn implied_floor(tool_name: &str) -> Option<Tier> {
        /// Verbs implying irreversible loss or a change to the world
        /// that cannot simply be edited back.
        const DESTRUCTIVE: &[&str] = &[
            "delete",
            "remove",
            "purge",
            "wipe",
            "erase",
            "destroy",
            "drop",
            "format",
            "truncate",
            "revoke",
            "uninstall",
            "kill",
            "terminate",
            "reset",
            "clear",
            "unpublish",
            "shred",
        ];
        /// Verbs implying state changes that are ordinarily reversible.
        const WRITE: &[&str] = &[
            "create", "add", "insert", "write", "update", "modify", "edit", "set", "put", "patch",
            "send", "post", "publish", "share", "move", "rename", "copy", "install", "enable",
            "disable", "start", "stop", "restart", "upload", "schedule", "invite", "assign",
            "archive", "import", "export", "sync",
        ];

        // Match on whole words, so `set` fires on `set_alarm` but not on
        // `get_settings` or `preset_list` — substring matching here would
        // make the floor fire on half the read tools in existence.
        let words: Vec<String> = tool_name
            .split(|c: char| !c.is_ascii_alphanumeric())
            .map(|w| w.to_ascii_lowercase())
            .filter(|w| !w.is_empty())
            .collect();

        if words.iter().any(|w| DESTRUCTIVE.contains(&w.as_str())) {
            return Some(Tier::Destructive);
        }
        if words.iter().any(|w| WRITE.contains(&w.as_str())) {
            return Some(Tier::Write);
        }
        None
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Read => "read",
            Tier::Write => "write",
            Tier::Destructive => "destructive",
        }
    }
}

/// Provenance tag on a chunk in the trigger chain (PLAN §4 rule 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    User,
    App(String),
    File,
    Mail,
    Screen,
    Web,
    /// Unrecognized tag — untrusted by construction (fail closed).
    Other(String),
}

impl Provenance {
    pub fn parse(s: &str) -> Provenance {
        match s {
            "user" => Provenance::User,
            "file" => Provenance::File,
            "mail" => Provenance::Mail,
            "screen" => Provenance::Screen,
            "web" => Provenance::Web,
            _ => match s.strip_prefix("app:") {
                Some(id) => Provenance::App(id.to_string()),
                None => Provenance::Other(s.to_string()),
            },
        }
    }

    /// Only direct user turns are trusted. App-provenance content is
    /// data an app forwarded (it may itself embed hostile mail/web
    /// text), so it escalates too.
    pub fn is_trusted(&self) -> bool {
        matches!(self, Provenance::User)
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Provenance::User => write!(f, "user"),
            Provenance::App(id) => write!(f, "app:{id}"),
            Provenance::File => write!(f, "file"),
            Provenance::Mail => write!(f, "mail"),
            Provenance::Screen => write!(f, "screen"),
            Provenance::Web => write!(f, "web"),
            Provenance::Other(s) => write!(f, "{s}"),
        }
    }
}

/// What the user must do before the call may execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confirmation {
    Silent,
    Chip,
    Modal,
}

impl Confirmation {
    pub fn for_tier(tier: Tier) -> Confirmation {
        match tier {
            Tier::Read => Confirmation::Silent,
            Tier::Write => Confirmation::Chip,
            Tier::Destructive => Confirmation::Modal,
        }
    }

    /// The lowest tier a tool with this name could honestly declare
    /// (issue #56). `None` when the name implies nothing.
    ///
    /// A manifest states its own tiers and nothing checked them, so an
    /// app could declare `delete_everything` as `read` and get SILENT
    /// execution — no confirmation, no undo journal entry, because undo
    /// is gated on `is_privileged`. The manifest is the ontology the
    /// whole tier machinery reasons over (ADR-0030 §5); a lie in it is
    /// not a mistake the rest of the system can recover from.
    ///
    /// Deliberately name-based and deliberately coarse. This cannot
    /// detect a *dishonest* tool — `harmless_helper` that deletes your
    /// mail is undetectable from a manifest — and it is not trying to.
    /// It catches the case where the name says one thing and the tier
    /// says another, which is the difference between a lie and a lie
    /// that also bothered to hide.
    pub fn implied_floor(tool_name: &str) -> Option<Tier> {
        /// Verbs implying irreversible loss or a change to the world
        /// that cannot simply be edited back.
        const DESTRUCTIVE: &[&str] = &[
            "delete",
            "remove",
            "purge",
            "wipe",
            "erase",
            "destroy",
            "drop",
            "format",
            "truncate",
            "revoke",
            "uninstall",
            "kill",
            "terminate",
            "reset",
            "clear",
            "unpublish",
            "shred",
        ];
        /// Verbs implying state changes that are ordinarily reversible.
        const WRITE: &[&str] = &[
            "create", "add", "insert", "write", "update", "modify", "edit", "set", "put", "patch",
            "send", "post", "publish", "share", "move", "rename", "copy", "install", "enable",
            "disable", "start", "stop", "restart", "upload", "schedule", "invite", "assign",
            "archive", "import", "export", "sync",
        ];

        // Match on whole words, so `set` fires on `set_alarm` but not on
        // `get_settings` or `preset_list` — substring matching here would
        // make the floor fire on half the read tools in existence.
        let words: Vec<String> = tool_name
            .split(|c: char| !c.is_ascii_alphanumeric())
            .map(|w| w.to_ascii_lowercase())
            .filter(|w| !w.is_empty())
            .collect();

        if words.iter().any(|w| DESTRUCTIVE.contains(&w.as_str())) {
            return Some(Tier::Destructive);
        }
        if words.iter().any(|w| WRITE.contains(&w.as_str())) {
            return Some(Tier::Write);
        }
        None
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Confirmation::Silent => "silent",
            Confirmation::Chip => "chip",
            Confirmation::Modal => "modal",
        }
    }
}

/// Resolved policy for one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub declared: Tier,
    pub effective: Tier,
    pub confirmation: Confirmation,
    /// True when the chain contained (or lacked) provenance such that
    /// the tier was escalated — surfaces in confirmation UI and Ledger.
    pub escalated: bool,
}

/// Resolve the confirmation requirement for a tool of `declared` tier
/// triggered by `chain`. An empty chain is an unknown origin: escalate.
pub fn resolve(declared: Tier, chain: &[Provenance]) -> Resolution {
    let untrusted = chain.is_empty() || chain.iter().any(|p| !p.is_trusted());
    let effective = if untrusted {
        declared.escalated()
    } else {
        declared
    };
    Resolution {
        declared,
        effective,
        confirmation: Confirmation::for_tier(effective),
        escalated: untrusted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> Vec<Provenance> {
        vec![Provenance::User]
    }

    #[test]
    fn trusted_chain_keeps_declared_tier() {
        let r = resolve(Tier::Read, &user());
        assert_eq!(r.confirmation, Confirmation::Silent);
        assert!(!r.escalated);
        assert_eq!(
            resolve(Tier::Write, &user()).confirmation,
            Confirmation::Chip
        );
        assert_eq!(
            resolve(Tier::Destructive, &user()).confirmation,
            Confirmation::Modal
        );
    }

    #[test]
    fn untrusted_provenance_escalates_one_tier() {
        for p in [
            Provenance::File,
            Provenance::Mail,
            Provenance::Screen,
            Provenance::Web,
            Provenance::App("org.example.App".into()),
            Provenance::Other("weird".into()),
        ] {
            let chain = vec![Provenance::User, p.clone()];
            let read = resolve(Tier::Read, &chain);
            assert_eq!(read.effective, Tier::Write, "read escalates on {p}");
            assert_eq!(read.confirmation, Confirmation::Chip);
            assert!(read.escalated);
            let write = resolve(Tier::Write, &chain);
            assert_eq!(write.confirmation, Confirmation::Modal);
            assert!(write.escalated);
        }
    }

    #[test]
    fn destructive_stays_modal_but_flags_escalation() {
        let r = resolve(Tier::Destructive, &[Provenance::Mail]);
        assert_eq!(r.effective, Tier::Destructive);
        assert_eq!(r.confirmation, Confirmation::Modal);
        assert!(r.escalated, "UI must render the untrusted-trigger warning");
    }

    #[test]
    fn empty_chain_fails_closed() {
        let r = resolve(Tier::Read, &[]);
        assert_eq!(
            r.confirmation,
            Confirmation::Chip,
            "unknown origin is untrusted origin"
        );
        assert!(r.escalated);
    }

    #[test]
    fn provenance_parse_round_trips_and_unknown_is_untrusted() {
        assert_eq!(Provenance::parse("user"), Provenance::User);
        assert!(Provenance::parse("user").is_trusted());
        assert_eq!(
            Provenance::parse("app:org.gnome.Calendar"),
            Provenance::App("org.gnome.Calendar".into())
        );
        for tag in ["file", "mail", "screen", "web", "app:x", "banana"] {
            assert!(
                !Provenance::parse(tag).is_trusted(),
                "{tag} must be untrusted"
            );
            assert_eq!(Provenance::parse(tag).to_string(), tag);
        }
    }
}
