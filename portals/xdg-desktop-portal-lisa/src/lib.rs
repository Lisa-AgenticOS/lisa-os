//! xdg-desktop-portal-lisa — the trust boundary (`docs/PLAN.md` §5.5,
//! §5.10; ADR-0008).
//!
//! Sandboxed apps never talk to the Lisa daemons directly (§4 rule 1):
//! this portal is the sole door. It attaches per-app identity (Flatpak
//! `.flatpak-info`, or peer-cred + `.desktop` mapping for host apps),
//! runs first-use consent ("always / only this time"), enforces per-app
//! quotas (requests/min, tokens/day), writes every decision and call to
//! the Ledger under the *real* app id, and proxies inference sessions to
//! `org.lisa.Inference1` so revoking a grant kills the live session.
//!
//! The D-Bus surface (`org.lisa.portal.Inference`, `org.lisa.portal.Grants`)
//! lives in [`portal`]; everything it decides with — identity, grants,
//! quotas, consent — is host-independent library code, unit-tested on any
//! dev host. Runtime registration on the session bus is Linux territory.

pub mod consent;
pub mod grants;
pub mod identity;
pub mod portal;
pub mod quota;
pub mod upstream;

/// The one scope M2 ships: talking to the system model at all.
/// Context scopes (`documents.read`, `mail.read`, `screen.once`, …)
/// arrive with the Context portal (M3) and reuse the same grant store.
pub const SCOPE_INFERENCE: &str = "inference";
