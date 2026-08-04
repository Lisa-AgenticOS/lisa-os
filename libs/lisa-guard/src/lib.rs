//! Deterministic guardrails for agent-issued actions (ADR-0029).
//!
//! Probabilistic reasoning inside, logical guardrails outside. Nothing in
//! this crate consults a model, reads a prompt, or depends on the model
//! cooperating: every answer is a pure function of the request, so the
//! policy can be tested exhaustively and cannot be talked out of.
//!
//! Three surfaces call it. [`contain`] is the filesystem boundary — the
//! agent reaches the directory it was given and nothing above it, symlinks
//! included. [`check_command`] and [`check_shell_line`] are the execution
//! boundary: the first for argv-form calls the forge harness makes with no
//! shell involved, the second for the free-form string `lisa suggest`
//! types into the user's prompt. [`judge_action`] is the **Agent Bus**
//! boundary (#251, #252): the same catalogue of refusals, applied to a
//! tool call rather than to a command line, so that a model which CALLS
//! `delete_everything("/")` meets the policy a model which TYPES
//! `rm -rf /` has met since ADR-0029.
//!
//! The [`Verdict::Deny`] class is not overridable *from inside*. A tier
//! system whose every level is reachable by clicking "yes" is a speed
//! bump; the point of a guardrail is the action that is simply not
//! available to the model.
//!
//! It is overridable from **outside** — by the person who owns the
//! machine, out-of-band, via [`Overrides`] (ADR-0030). The boundary this
//! crate draws is around the probabilistic system, and the human is on
//! the far side of it along with the guard. The invariant that keeps
//! that honest is that the mechanism is unreachable from any tool call.

mod action;
mod command;
mod overrides;
mod path;
mod protections;
mod rules;
mod shell;

pub use action::{
    Action, ActionVerdict, BUS_RULES, Class, Grant, HARD_NO_RULES, Trigger, judge as judge_action,
};
pub use command::{ALLOWED_COMMANDS, Allowlist, check_command, check_command_advisory};
pub use overrides::{Overrides, active as active_overrides, overrides_path};
pub use path::{ContainError, contain, write_contained};
pub use protections::Protections;
pub use shell::{Invocation, check_shell_line};

use std::fmt;

/// What the policy says about one requested action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Proceed.
    Allow,
    /// A human has to say yes first. `lisa do --yes` may satisfy this.
    Confirm { rule: &'static str, reason: String },
    /// Refused. No confirmation, no flag, and no prompt overrides it.
    Deny { rule: &'static str, reason: String },
}

impl Verdict {
    pub fn confirm(rule: &'static str, reason: impl Into<String>) -> Self {
        Self::Confirm {
            rule,
            reason: reason.into(),
        }
    }

    pub fn deny(rule: &'static str, reason: impl Into<String>) -> Self {
        Self::Deny {
            rule,
            reason: reason.into(),
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, Verdict::Allow)
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, Verdict::Deny { .. })
    }

    /// Whether a human (or `--yes`) can still let this through.
    pub fn is_overridable(&self) -> bool {
        matches!(self, Verdict::Confirm { .. })
    }

    /// The rule id that produced this verdict, for the ledger and for
    /// tests that assert *why* something was stopped rather than only
    /// that it was.
    pub fn rule(&self) -> Option<&'static str> {
        match self {
            Verdict::Allow => None,
            Verdict::Confirm { rule, .. } | Verdict::Deny { rule, .. } => Some(rule),
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Verdict::Allow => None,
            Verdict::Confirm { reason, .. } | Verdict::Deny { reason, .. } => Some(reason),
        }
    }

    fn severity(&self) -> u8 {
        match self {
            Verdict::Allow => 0,
            Verdict::Confirm { .. } => 1,
            Verdict::Deny { .. } => 2,
        }
    }

    /// The stricter of two verdicts. A command line is judged by its
    /// worst segment — `ls && rm -rf /` is not half safe.
    pub fn worst(self, other: Self) -> Self {
        if other.severity() > self.severity() {
            other
        } else {
            self
        }
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Allow => write!(f, "allowed"),
            Verdict::Confirm { rule, reason } => write!(f, "needs confirmation [{rule}]: {reason}"),
            Verdict::Deny { rule, reason } => write!(f, "refused [{rule}]: {reason}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worst_wins_and_deny_is_final() {
        let allow = Verdict::Allow;
        let confirm = Verdict::confirm("r", "maybe");
        let deny = Verdict::deny("r", "no");

        assert_eq!(allow.clone().worst(confirm.clone()), confirm);
        assert_eq!(confirm.clone().worst(deny.clone()), deny);
        assert_eq!(deny.clone().worst(confirm), deny);

        assert!(Verdict::confirm("r", "x").is_overridable());
        assert!(!Verdict::deny("r", "x").is_overridable());
        assert!(!Verdict::Allow.is_overridable());
    }
}
