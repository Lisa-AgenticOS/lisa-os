//! Consent (`docs/PLAN.md` §5.5): first-use grant with "always / only
//! this time", remembered denies, fail-closed when no dialog can be
//! shown. The portal decides *policy* here; the *pixels* belong to the
//! shell, reached over `org.lisa.impl.portal.Consent` (the impl-portal
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
/// `org.lisa.impl.portal.Consent` at `/org/lisa/impl/portal/consent`
/// with `AskConsent(app_id s, app_kind s, scope s) -> (allow b, remember b)`.
/// Any error (service absent, dialog dismissed, timeout) → `None` →
/// fail closed.
pub struct DbusConsentUi {
    conn: zbus::Connection,
}

impl DbusConsentUi {
    pub const BUS_NAME: &'static str = "org.lisa.Shell";
    pub const PATH: &'static str = "/org/lisa/impl/portal/consent";
    pub const INTERFACE: &'static str = "org.lisa.impl.portal.Consent";

    pub fn new(conn: zbus::Connection) -> Self {
        Self { conn }
    }
}

impl ConsentUi for DbusConsentUi {
    fn ask(&self, app: &AppIdentity, scope: &str) -> BoxFuture<'_, Option<ConsentReply>> {
        let app_id = app.app_id.clone();
        let app_kind = match app.kind {
            crate::identity::IdentityKind::Flatpak => "flatpak",
            crate::identity::IdentityKind::Host => "host",
        };
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
            Some(ConsentReply {
                allow: false,
                remember: false,
            })
            | None => Authorization::Denied { record: None },
        },
    }
}

/// Whether [`authorize`] needs a dialog at all (lets the caller skip
/// the UI round-trip for remembered decisions).
pub fn needs_prompt(effective: Effective) -> bool {
    effective == Effective::Unset
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

    #[test]
    fn no_dialog_backend_fails_closed() {
        assert_eq!(
            authorize(Effective::Unset, None),
            Authorization::Denied { record: None }
        );
    }
}
