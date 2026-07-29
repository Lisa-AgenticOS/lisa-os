//! Consent (`docs/PLAN.md` §5.5): first-use grant with "always / only
//! this time", remembered denies, fail-closed when no dialog can be
//! shown. The portal decides *policy* here; the *pixels* belong to the
//! shell, reached over `dev.lisaos.impl.portal.Consent` (the impl-portal
//! split upstream xdg-desktop-portal uses — see ADR-0008). The M4 shell
//! provides that dialog service; until it exists, first-use requests are
//! denied, never silently allowed.

use crate::grants::{Effective, GrantAction};
use crate::identity::AppIdentity;
use futures::future::BoxFuture;

/// What the user answered in the consent dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentReply {
    pub allow: bool,
    /// "Always" / "never" vs "only this time".
    pub remember: bool,
}

/// Shows (or refuses to show) a consent dialog. `None` means no dialog
/// backend was reachable — the caller must fail closed.
pub trait ConsentUi: Send + Sync {
    fn ask(&self, app: &AppIdentity, scope: &str) -> BoxFuture<'_, Option<ConsentReply>>;
}

/// Fixed answer — tests and explicit dev modes (`--consent allow|deny`).
pub struct StaticConsent(pub Option<ConsentReply>);

impl StaticConsent {
    pub fn allow_always() -> Self {
        Self(Some(ConsentReply {
            allow: true,
            remember: true,
        }))
    }

    pub fn allow_once() -> Self {
        Self(Some(ConsentReply {
            allow: true,
            remember: false,
        }))
    }

    pub fn deny() -> Self {
        Self(Some(ConsentReply {
            allow: false,
            remember: false,
        }))
    }

    /// No dialog backend — what a headless system looks like.
    pub fn unavailable() -> Self {
        Self(None)
    }
}

impl ConsentUi for StaticConsent {
    fn ask(&self, _app: &AppIdentity, _scope: &str) -> BoxFuture<'_, Option<ConsentReply>> {
        let reply = self.0;
        Box::pin(async move { reply })
    }
}

/// Consent dialog over the session bus: the shell serves
/// `dev.lisaos.impl.portal.Consent` at `/dev/lisaos/impl/portal/consent`
/// with `AskConsent(app_id s, app_kind s, scope s) -> (allow b, remember b)`.
/// Any error (service absent, dialog dismissed, timeout) → `None` →
/// fail closed.
pub struct DbusConsentUi {
    conn: zbus::Connection,
}

impl DbusConsentUi {
    pub const BUS_NAME: &'static str = "dev.lisaos.Shell";
    pub const PATH: &'static str = "/dev/lisaos/impl/portal/consent";
    pub const INTERFACE: &'static str = "dev.lisaos.impl.portal.Consent";

    pub fn new(conn: zbus::Connection) -> Self {
        Self { conn }
    }
}

impl ConsentUi for DbusConsentUi {
    fn ask(&self, app: &AppIdentity, scope: &str) -> BoxFuture<'_, Option<ConsentReply>> {
        let app_id = app.app_id.clone();
        let app_kind = app.kind.as_str();
        let scope = scope.to_string();
        Box::pin(async move {
            let proxy = zbus::Proxy::new(&self.conn, Self::BUS_NAME, Self::PATH, Self::INTERFACE)
                .await
                .ok()?;
            let reply = proxy
                .call_method("AskConsent", &(app_id, app_kind, scope))
                .await
                .ok()?;
            let (allow, remember): (bool, bool) = reply.body().deserialize().ok()?;
            Some(ConsentReply { allow, remember })
        })
    }
}

/// The authorization verdict for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authorization {
    Granted { record: Option<GrantAction> },
    Denied { record: Option<GrantAction> },
}

/// Pure policy: combine the stored effective state with a (possible)
/// dialog answer. Remembered decisions never re-prompt; unset + no
/// dialog answer fails closed.
pub fn authorize(effective: Effective, reply: Option<ConsentReply>) -> Authorization {
    match effective {
        Effective::Allowed => Authorization::Granted { record: None },
        Effective::Denied => Authorization::Denied { record: None },
        Effective::Unset => match reply {
            Some(ConsentReply {
                allow: true,
                remember: true,
            }) => Authorization::Granted {
                record: Some(GrantAction::Allow),
            },
            Some(ConsentReply {
                allow: true,
                remember: false,
            }) => Authorization::Granted {
                record: Some(GrantAction::AllowOnce),
            },
            Some(ConsentReply {
                allow: false,
                remember: true,
            }) => Authorization::Denied {
                record: Some(GrantAction::Deny),
            },
            // A refusal the user did not ask to remember — and a dialog
            // that was dismissed, timed out, or never appeared. Both are
            // still recorded (#113): the old code wrote nothing, so an
            // app could ask again immediately, and again, until a
            // mis-click. The record does not change effective state; it
            // is what [`PromptPolicy`] counts.
            Some(ConsentReply {
                allow: false,
                remember: false,
            })
            | None => Authorization::Denied {
                record: Some(GrantAction::DenyOnce),
            },
        },
    }
}

/// Whether [`authorize`] needs a dialog at all (lets the caller skip
/// the UI round-trip for remembered decisions).
pub fn needs_prompt(effective: Effective) -> bool {
    effective == Effective::Unset
}

/// How often an app may put a consent dialog in front of the user
/// (issue #113).
///
/// ADR-0030's test — *is the boundary reachable from inside?* — has an
/// answer that is easy to miss: a dialog is reachable from inside, one
/// click at a time. An app that may ask without limit does not need to
/// defeat consent, only to outlast the person answering. So refusals are
/// counted, and after `max_refusals` within `window`, asking stops
/// working for the rest of the window.
///
/// The cooldown deliberately does *not* become a remembered denial. The
/// user said "not now", not "never", and turning hesitation into a
/// permanent state would be the portal overriding them — the other
/// failure mode, and the one people cannot undo without going to look
/// for a setting they do not know exists.
#[derive(Debug, Clone, Copy)]
pub struct PromptPolicy {
    pub max_refusals: u32,
    pub window_ms: i64,
}

impl Default for PromptPolicy {
    fn default() -> Self {
        // Three refusals in a quarter of an hour. A person clicking "not
        // now" three times has answered; a fourth dialog is nagging.
        Self {
            max_refusals: 3,
            window_ms: 15 * 60 * 1000,
        }
    }
}

impl PromptPolicy {
    /// The start of the window to count refusals from.
    ///
    /// Never negative: refusal timestamps are milliseconds since the
    /// epoch, so a negative floor is not "earlier", it is a nonsense
    /// value being compared against real ones.
    pub fn window_start(&self, now_ms: i64) -> i64 {
        now_ms.saturating_sub(self.window_ms).max(0)
    }

    /// Whether an app that has been refused `refusals` times in the
    /// window may raise another dialog.
    pub fn may_prompt(&self, refusals: u32) -> bool {
        refusals < self.max_refusals
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembered_decisions_skip_the_dialog() {
        assert!(!needs_prompt(Effective::Allowed));
        assert!(!needs_prompt(Effective::Denied));
        assert!(needs_prompt(Effective::Unset));
        assert_eq!(
            authorize(Effective::Allowed, None),
            Authorization::Granted { record: None }
        );
        assert_eq!(
            authorize(Effective::Denied, None),
            Authorization::Denied { record: None }
        );
    }

    #[test]
    fn first_use_always_records_a_persistent_grant() {
        assert_eq!(
            authorize(
                Effective::Unset,
                Some(ConsentReply {
                    allow: true,
                    remember: true
                })
            ),
            Authorization::Granted {
                record: Some(GrantAction::Allow)
            }
        );
    }

    #[test]
    fn only_this_time_grants_without_persisting() {
        assert_eq!(
            authorize(
                Effective::Unset,
                Some(ConsentReply {
                    allow: true,
                    remember: false
                })
            ),
            Authorization::Granted {
                record: Some(GrantAction::AllowOnce)
            }
        );
    }

    #[test]
    fn deny_with_remember_persists_the_refusal() {
        assert_eq!(
            authorize(
                Effective::Unset,
                Some(ConsentReply {
                    allow: false,
                    remember: true
                })
            ),
            Authorization::Denied {
                record: Some(GrantAction::Deny)
            }
        );
    }

    /// Fails closed — and, since #113, leaves a trace. A headless system
    /// that answers `None` forever would otherwise let an app spin on
    /// OpenSession with nothing anywhere noticing.
    #[test]
    fn no_dialog_backend_fails_closed() {
        assert_eq!(
            authorize(Effective::Unset, None),
            Authorization::Denied {
                record: Some(GrantAction::DenyOnce)
            }
        );
    }

    /// "No, not now" must be recorded without becoming "never".
    #[test]
    fn a_once_denial_is_recorded_but_stays_unset() {
        assert_eq!(
            authorize(
                Effective::Unset,
                Some(ConsentReply {
                    allow: false,
                    remember: false
                })
            ),
            Authorization::Denied {
                record: Some(GrantAction::DenyOnce)
            }
        );
        assert!(!GrantAction::DenyOnce.is_persistent());
    }

    #[test]
    fn prompting_stops_after_repeated_refusals_within_the_window() {
        let policy = PromptPolicy::default();
        assert!(policy.may_prompt(0));
        assert!(policy.may_prompt(policy.max_refusals - 1));
        assert!(!policy.may_prompt(policy.max_refusals));
        assert!(!policy.may_prompt(policy.max_refusals + 100));
    }

    /// The window slides rather than accumulating forever — an app
    /// refused this morning is not muted all day.
    #[test]
    fn the_refusal_window_is_relative_to_now() {
        let policy = PromptPolicy::default();
        assert_eq!(policy.window_start(policy.window_ms), 0);
        assert_eq!(
            policy.window_start(policy.window_ms * 3),
            policy.window_ms * 2
        );
        // And it never runs off the bottom on a machine with a bad clock.
        assert_eq!(policy.window_start(0), 0);
    }
}
