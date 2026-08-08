//! Who is a person's prompt surface, and who hosts a model — the shared
//! definition both daemons must agree on (#306, ADR-0064).
//!
//! # Why this is one module and not two
//!
//! The decision "is this caller a surface a person typed into" gates the
//! most powerful class a run can get (`Trigger::Prompt`: file tools, the
//! `run_command` family, write-tier bus calls, `user` provenance,
//! owner-only memory). It used to be split across two daemons and a bus
//! call: harnessd knew the well-known *names*, agentd knew the
//! *programs* and answered `IsPromptSurface` over D-Bus because harnessd
//! — sandboxed into a user namespace — could not read `/proc/<peer>/exe`
//! (#161).
//!
//! The close-replay of #306 showed the bus call was the bug one hop
//! along: `dev.lisaos.Agent1` is itself a claimable name, so a squatter
//! could be its own oracle. ADR-0064's fix takes harnessd out of the
//! namespace so it reads `/proc` itself — at which point both daemons
//! ask the SAME question and the answer must have exactly one
//! definition, or the two copies drift into a "human typed this" the
//! attacker controls. That single definition is here.
//!
//! Everything is a pure function of `(same_user, exe)` — the two facts
//! the transport supplies (`Peer::is_same_user_as_us`,
//! `exe_of_peer`) — so every case is a unit test, not a claim.

use crate::manager::{may_manage, resolve_managers};
use std::path::{Path, PathBuf};

/// Programs that host a model, as absolute executable paths.
///
/// A model host is the process the model runs *inside*; it is never a
/// prompt surface and never a consent surface, whatever any other list
/// says. `agentd` trusts `user` provenance from these because the run
/// they front for was gated by [`is_prompt_surface`] here.
pub const MODEL_HOSTS: [&str; 1] = ["/usr/bin/lisa-harnessd"];

/// Programs that may be a surface a person types into, as absolute
/// executable paths.
///
/// # The limit, stated rather than wished away
///
/// The Assistant is `Exec=/usr/bin/lisa-app assistant/lisa-assistant.js
/// --gapplication-service`, and `lisa-app` ends in `exec gjs -m …`, so
/// the kernel's answer for the shipped Assistant is `/usr/bin/gjs`
/// (a symlink to `gjs-console`; `resolve_managers` canonicalises, so
/// either spelling matches). An allowlist holding an *interpreter*
/// authorises every program that interpreter can run:
///
/// - it refuses `/usr/bin/lisa-harnessd` — the process the model runs
///   inside, the #306 escalation and ADR-0029's whole threat model;
/// - it refuses every compiled squatter, and the `python3` one the #306
///   audit actually ran;
/// - it does **not** refuse a hostile GJS script.
///
/// `/usr/bin/lisa-assistant` is listed for the day the Assistant has an
/// executable of its own — at which point passing this check means
/// running our code. That is packaging work (ADR-0064's limits), and
/// `resolve_managers` drops paths that do not exist, so shipping the
/// binary is the whole of the change on this side.
pub const PROMPT_SURFACE_PROGRAMS: [&str; 2] = ["/usr/bin/lisa-assistant", "/usr/bin/gjs"];

/// Does this executable host a model?
///
/// Fails closed: a caller we cannot place, or one that is not our user,
/// does not host a model.
pub fn hosts_a_model(same_user: bool, exe: Option<&Path>) -> bool {
    let hosts: Vec<PathBuf> = MODEL_HOSTS.iter().map(PathBuf::from).collect();
    may_manage(same_user, exe, &resolve_managers(&hosts)).is_ok()
}

/// Is this caller a surface a person types into?
///
/// Fails CLOSED: an unreadable or foreign peer is not a prompt surface,
/// and the cost of being wrong is a run classed `Trigger::Event`, whose
/// content is never trusted — the safe direction.
///
/// A model host is never a prompt surface, checked first and separately
/// from the allowlist. That is belt-and-braces over the lists being
/// disjoint: the two being edited apart is exactly the drift that turns
/// the loop into its own "human typed this" (#306's laundering chain),
/// and `MODEL_HOSTS ∩ PROMPT_SURFACE_PROGRAMS == ∅` is asserted below.
pub fn is_prompt_surface(same_user: bool, exe: Option<&Path>) -> bool {
    if hosts_a_model(same_user, exe) {
        return false;
    }
    let programs: Vec<PathBuf> = PROMPT_SURFACE_PROGRAMS.iter().map(PathBuf::from).collect();
    may_manage(same_user, exe, &resolve_managers(&programs)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_foreign_user_is_neither() {
        let exe = Some(Path::new("/usr/bin/gjs"));
        assert!(!is_prompt_surface(false, exe));
        assert!(!hosts_a_model(
            false,
            Some(Path::new("/usr/bin/lisa-harnessd"))
        ));
    }

    #[test]
    fn an_unidentified_peer_is_neither() {
        assert!(!is_prompt_surface(true, None));
        assert!(!hosts_a_model(true, None));
    }

    /// The invariant the disjointness of the two lists rests on: the
    /// model's own host can never be classed a prompt surface, so a run
    /// the model started can never be laundered into the class a person
    /// typed (#306). Checked against the real path rather than trusting
    /// the two `const` arrays not to overlap.
    #[test]
    fn a_model_host_is_never_a_prompt_surface() {
        // Only meaningful where the path resolves (the model host is
        // installed); on a dev host it does not, and the check still
        // must not classify it as a surface.
        assert!(!is_prompt_surface(true, Some(Path::new(MODEL_HOSTS[0]))));
        // And the two lists share no member.
        for host in MODEL_HOSTS {
            assert!(
                !PROMPT_SURFACE_PROGRAMS.contains(&host),
                "{host} is on both lists — a model host would be a prompt surface"
            );
        }
    }
}
