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
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            requests_per_min: 120,
            tokens_per_day: 500_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuotaExceeded {
    #[error("per-app request rate exceeded (requests/min quota)")]
    Rate,
    #[error("per-app daily token budget exceeded (tokens/day quota)")]
    Tokens,
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

/// Token-budget check against persisted daily usage.
pub fn check_tokens(used_today: i64, cfg: &QuotaConfig) -> Result<(), QuotaExceeded> {
    if used_today >= cfg.tokens_per_day {
        Err(QuotaExceeded::Tokens)
    } else {
        Ok(())
    }
}

/// Coarse token estimate (whitespace words) — used for quota accounting
/// until `inferenced` emits real TokenUsage per session (M2 backlog,
/// see `daemons/inferenced/src/dbus.rs`). Over-counting is preferred to
/// under-counting for an anti-abuse bound.
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

    #[test]
    fn token_budget_refuses_at_the_cap() {
        let cfg = cfg(10, 100);
        assert!(check_tokens(99, &cfg).is_ok());
        assert_eq!(check_tokens(100, &cfg), Err(QuotaExceeded::Tokens));
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
