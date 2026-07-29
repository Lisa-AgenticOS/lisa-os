//! Per-app quotas (`docs/PLAN.md` §5.5): requests/min and tokens/day.
//! "Generous; anti-abuse, not monetization" — the defaults exist to stop
//! a runaway loop from monopolizing the machine, not to meter usage.
//!
//! Requests/min is a sliding in-memory window (losing it on restart is
//! harmless at this granularity); tokens/day persists via
//! [`crate::grants::GrantStore`] so restarts don't reset budgets. All
//! logic takes explicit `now` seconds — deterministic under test.

use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Copy)]
pub struct QuotaConfig {
    pub requests_per_min: u32,
    pub tokens_per_day: i64,
    /// How many portal sessions one app may hold open at once.
    ///
    /// Each session is an upstream daemon session, a passed file
    /// descriptor and a registered D-Bus object, and `OpenSession` used
    /// to be subject to no limit whatsoever (#111) — 50 in a row, with a
    /// requests/min quota of 1 configured, all admitted.
    pub max_sessions_per_app: usize,
    /// What a generation is charged for its output when nothing says
    /// how long it will be.
    ///
    /// See [`output_reservation`]: the portal cannot observe output, so
    /// it reserves rather than measures.
    pub assumed_output_tokens: i64,
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            requests_per_min: 120,
            tokens_per_day: 500_000,
            max_sessions_per_app: 16,
            assumed_output_tokens: 2_048,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuotaExceeded {
    #[error("per-app request rate exceeded (requests/min quota)")]
    Rate,
    #[error("per-app daily token budget exceeded (tokens/day quota)")]
    Tokens,
    #[error("too many open sessions for this app")]
    Sessions,
}

/// Day bucket key for persisted token accounting.
pub fn day_key(now_secs: u64) -> String {
    format!("day-{}", now_secs / 86_400)
}

/// In-memory sliding-window request counter, per app.
#[derive(Default)]
pub struct QuotaBook {
    windows: HashMap<String, VecDeque<u64>>,
}

impl QuotaBook {
    /// Admit (and record) one request at `now_secs`, or refuse.
    pub fn check_request(
        &mut self,
        app_id: &str,
        cfg: &QuotaConfig,
        now_secs: u64,
    ) -> Result<(), QuotaExceeded> {
        let window = self.windows.entry(app_id.to_string()).or_default();
        while let Some(&front) = window.front() {
            if now_secs.saturating_sub(front) >= 60 {
                window.pop_front();
            } else {
                break;
            }
        }
        if window.len() >= cfg.requests_per_min as usize {
            return Err(QuotaExceeded::Rate);
        }
        window.push_back(now_secs);
        Ok(())
    }
}

/// What one generation costs the daily budget: its prompt, plus a
/// reservation for the output it is about to produce.
///
/// **A reservation, not a measurement — and the difference is the
/// honest part.** Tokens stream from `inferenced` straight down the
/// app's file descriptor; the portal never sees them, so it cannot
/// count what came back. Before this it counted nothing at all (#114),
/// which meant an app could drive arbitrarily long generations at the
/// price of a short prompt.
///
/// So the budget is charged up front for the largest output the request
/// could produce: `max_tokens` when the caller states one — which is the
/// caller *lowering* its own bill, so there is no incentive to lie
/// upward and lying downward does not buy more output — and
/// `assumed_output_tokens` when it does not.
///
/// Real accounting replaces this when `inferenced` reports TokenUsage
/// per session (M2 backlog, `daemons/inferenced/src/dbus.rs`). Until
/// then the number is deliberately an over-estimate: this is an
/// anti-abuse bound, not a bill.
pub fn generation_cost(prompt: &str, max_tokens: Option<i64>, cfg: &QuotaConfig) -> i64 {
    estimate_tokens(prompt).saturating_add(output_reservation(max_tokens, cfg))
}

/// The output half of [`generation_cost`].
pub fn output_reservation(max_tokens: Option<i64>, cfg: &QuotaConfig) -> i64 {
    match max_tokens {
        // A stated ceiling is honoured, but never as a way to reserve
        // *less* than nothing, and never above the day's whole budget —
        // a caller asking for a billion tokens should be refused by the
        // budget, not overflow the arithmetic that enforces it.
        Some(n) if n > 0 => n.min(cfg.tokens_per_day),
        Some(_) => 0,
        None => cfg.assumed_output_tokens,
    }
}

/// Coarse token estimate (whitespace words). Over-counting is preferred
/// to under-counting for an anti-abuse bound.
pub fn estimate_tokens(text: &str) -> i64 {
    text.split_whitespace().count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(rpm: u32, tpd: i64) -> QuotaConfig {
        QuotaConfig {
            requests_per_min: rpm,
            tokens_per_day: tpd,
            ..QuotaConfig::default()
        }
    }

    #[test]
    fn rate_limit_refuses_then_recovers_as_the_window_slides() {
        let mut book = QuotaBook::default();
        let cfg = cfg(2, 1000);
        assert!(book.check_request("app.a", &cfg, 100).is_ok());
        assert!(book.check_request("app.a", &cfg, 110).is_ok());
        assert_eq!(
            book.check_request("app.a", &cfg, 120),
            Err(QuotaExceeded::Rate)
        );
        // 60 s after the first request it falls out of the window.
        assert!(book.check_request("app.a", &cfg, 161).is_ok());
    }

    #[test]
    fn rate_limit_is_per_app() {
        let mut book = QuotaBook::default();
        let cfg = cfg(1, 1000);
        assert!(book.check_request("app.a", &cfg, 100).is_ok());
        assert!(book.check_request("app.b", &cfg, 100).is_ok());
        assert_eq!(
            book.check_request("app.a", &cfg, 101),
            Err(QuotaExceeded::Rate)
        );
    }

    /// Issue #114's second half: output was never charged at all, so a
    /// two-word prompt could drive an unbounded generation for two
    /// tokens of budget.
    #[test]
    fn a_generation_is_charged_for_its_output_too() {
        let cfg = cfg(10, 100_000);
        let prompt = "write me a novel";
        assert_eq!(estimate_tokens(prompt), 4);
        assert_eq!(
            generation_cost(prompt, None, &cfg),
            4 + cfg.assumed_output_tokens
        );
        // A stated ceiling is what gets charged instead.
        assert_eq!(generation_cost(prompt, Some(50), &cfg), 54);
    }

    /// The reservation must not be turnable into a discount or an
    /// overflow: zero and negative ceilings reserve nothing (the prompt
    /// is still charged), and an absurd one is clamped to the day.
    #[test]
    fn a_stated_ceiling_cannot_be_gamed() {
        let cfg = cfg(10, 1_000);
        assert_eq!(output_reservation(Some(0), &cfg), 0);
        assert_eq!(output_reservation(Some(-5), &cfg), 0);
        assert_eq!(output_reservation(Some(i64::MAX), &cfg), cfg.tokens_per_day);
        assert_eq!(
            generation_cost("a b c", Some(i64::MAX), &cfg),
            3 + cfg.tokens_per_day,
            "clamped, and still far past a 1000-token budget — refused, not overflowed"
        );
    }

    #[test]
    fn day_key_rolls_over_at_midnight() {
        assert_eq!(day_key(0), "day-0");
        assert_eq!(day_key(86_399), "day-0");
        assert_eq!(day_key(86_400), "day-1");
    }

    #[test]
    fn token_estimate_counts_words() {
        assert_eq!(estimate_tokens("hello  world\nagain"), 3);
        assert_eq!(estimate_tokens(""), 0);
    }
}
