//! xdg-desktop-portal-lisa — the trust boundary (`docs/PLAN.md` §5.5,
//! §5.10; ADR-0008).
//!
//! Sandboxed apps never talk to the Lisa daemons directly (§4 rule 1):
//! this portal is the sole door. It attaches per-app identity (Flatpak
//! `.flatpak-info`, or the caller's executable matched against installed
//! `.desktop` files), runs first-use consent ("always / only this time"),
//! enforces per-app quotas (requests/min, tokens/day, open sessions),
//! writes every decision and call to the Ledger under the *real* app id,
//! and proxies inference sessions to `dev.lisaos.Inference1` so revoking
//! a grant kills the live session.
//!
//! # Everything here rests on knowing who is calling
//!
//! An adversarial review found six ways through this door (#106, #107,
//! #108, #111, #113, #114) and five of them were the same shape: a check
//! whose input the caller controlled. Host identity came from
//! `/proc/<pid>/comm`, which a process sets itself; grant management was
//! guarded by a comment; session objects trusted whoever held a path
//! anyone could guess. So caller identity comes from the transport now
//! (`lisa_peer`, ADR-0033), and the things that depend on it —
//! [`identity`], `lisa_peer::manager`, session ownership in [`portal`] — take it
//! as an argument rather than deriving it from the message.
//!
//! Who may *manage* grants is `lisa_peer::manager` — shared with
//! `remoted`, which had the same hole on its own management plane
//! (#99). One rule, one place.
//!
//! The D-Bus surface (`dev.lisaos.portal.Inference`, `dev.lisaos.portal.Grants`)
//! lives in [`portal`]; everything it decides with — identity, grants,
//! quotas, consent, who may manage — is host-independent library code,
//! unit-tested on any dev host. Runtime registration on the session bus
//! is Linux territory, and so is `/proc`: the exe-based half of identity
//! is exercised against a live bus in CI (`tests/bus.rs`).

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
