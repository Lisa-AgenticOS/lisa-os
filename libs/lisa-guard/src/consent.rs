//! Who may *answer* a parked confirmation (ADR-0030 §2, ADR-0033).
//!
//! The rest of this crate judges what an action IS. This module judges
//! something else entirely: once the bus has parked a privileged call,
//! **which peer is allowed to release it**. That question turned out to
//! be the load-bearing one the moment an agent loop was offered a
//! write-tier tool, because the peer most eager to release the call is
//! the process hosting the model that asked for it.
//!
//! # Why this is here and not in agentd
//!
//! Two reasons, both ADR-0030 §5. First, the decision must be a pure
//! function of transport-derived facts so it can be tested
//! exhaustively — agentd's version was three `if`s inside a method that
//! also held a mutex over the pending map, and the only way to exercise
//! it was to stand up a bus. Second, a refusal needs a **stable rule
//! id**: the ontology the outside layer reasons over is the one a
//! person reads in the Ledger and looks up in `lisa guard list`. A
//! refusal with no id is a refusal nobody can search for.
//!
//! # The rule that matters
//!
//! [`Approval::requester_hosts_a_model`] is not a claim. agentd derives
//! it from `/proc/<pid>/exe` (ADR-0033: peer credentials, never `comm`,
//! never anything in the message), and the model inside that process
//! cannot change its own executable. So:
//!
//! > **A process that hosts a model may never approve a call it made.**
//!
//! Not at destructive tier, not at write tier, not when no dialog is
//! running, not when the dialog will not start. Every one of those was
//! previously an exemption somewhere, and each exemption was reachable
//! from inside — which is the test ADR-0030 §2 says a guardrail has to
//! pass. What the requester keeps is the right to **withdraw** its own
//! call, because withdrawal causes no action.

use crate::HARD_NO_RULES;

/// The confirmation the bus resolved for a parked call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmClass {
    /// An inline chip — the app's own affordance, historically
    /// answerable by the app that drew it.
    Chip,
    /// A modal with a typed diff. Destructive tier, or anything the
    /// provenance rules escalated into it.
    Modal,
}

/// One attempt to answer a parked call, as the **transport** describes
/// it. Not one field here is read from the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Approval {
    /// `true` = release the call, `false` = withdraw it.
    pub approve: bool,
    /// The answering peer is the peer that parked this call
    /// (`Owner::allows`, the broker-assigned unique name).
    pub is_requester: bool,
    /// The answering peer currently owns `dev.lisaos.Consent1` — the
    /// broker's answer to `GetNameOwner`, which a sender cannot forge.
    pub owns_consent_name: bool,
    /// The peer that PARKED the call runs a program that hosts a model
    /// (`/proc/<pid>/exe` against agentd's model-host list).
    pub requester_hosts_a_model: bool,
    /// What the bus decided this call needs.
    pub class: ConfirmClass,
    /// There is a message bus at all. `false` is point-to-point, where
    /// requester and answerer are the same peer by construction and no
    /// separation exists to enforce. Decided by whether the transport
    /// gave the connection a unique name, never by a caller.
    pub brokered: bool,
}

/// What the bus should do with an [`Approval`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalVerdict {
    /// Go ahead.
    Allow,
    /// The answerer has no business with this call at all. Deliberately
    /// carries no rule and no reason: it is the one refusal that must
    /// reveal nothing, because telling a stranger *why* they cannot
    /// answer call 41 confirms that call 41 exists (#93, #131).
    NotYours,
    /// Refused, with an id a person can look up.
    Refused {
        rule: &'static str,
        reason: &'static str,
    },
}

impl ApprovalVerdict {
    pub fn rule(&self) -> Option<&'static str> {
        match self {
            ApprovalVerdict::Refused { rule, .. } => Some(rule),
            _ => None,
        }
    }

    /// Is this one of the refusals no dialog and no setting can lift?
    pub fn is_hard_no(&self) -> bool {
        self.rule().is_some_and(|r| HARD_NO_RULES.contains(&r))
    }
}

/// The whole decision, as a pure function of what the transport said.
///
/// Read it as three questions in order, because the order is the
/// policy:
///
/// 1. **Is this peer party to the call at all?** Only the requester and
///    an *independent* consent surface are. Independence is a property
///    of the pair, never of either alone — a process that both hosts
///    the model and owns the consent name is one process wearing two
///    hats, which is #145.
/// 2. **Is this a withdrawal?** Then yes, always. Taking your own call
///    back causes nothing to happen, and a guardrail that stopped it
///    would be aimed at the wrong side of ADR-0030's line.
/// 3. **Is the requester a model host?** Then only the independent
///    surface may approve — at every tier. Otherwise the historical
///    split stands: a modal needs the surface, a chip is the app's own
///    inline affordance.
pub fn judge_approval(a: &Approval) -> ApprovalVerdict {
    let independent_surface = a.owns_consent_name && !a.is_requester;
    if !a.is_requester && !independent_surface {
        return ApprovalVerdict::NotYours;
    }
    if !a.approve {
        // Withdrawal. The requester may always take its own call back,
        // and the surface may always refuse one.
        return ApprovalVerdict::Allow;
    }
    if independent_surface {
        return ApprovalVerdict::Allow;
    }

    // From here the answerer IS the requester, approving its own call.
    if a.requester_hosts_a_model {
        // No broker exemption, deliberately. p2p is the transport
        // agentd's own tests use and `main.rs` never builds; a model
        // host reaching this line on one would still be a model
        // approving itself, and there is no session in which that is
        // the intended behaviour.
        return ApprovalVerdict::Refused {
            rule: SELF_APPROVAL,
            reason: "the process that asked for this call is the process running \
                     the model, so approving it here would be the model approving \
                     itself — only the desktop consent surface can say yes",
        };
    }
    if a.class == ConfirmClass::Modal && a.brokered {
        return ApprovalVerdict::Refused {
            rule: NO_SURFACE,
            reason: "this call is destructive-tier and no independent consent \
                     surface answered it — the peer that asked for it may only \
                     withdraw it",
        };
    }
    ApprovalVerdict::Allow
}

/// A model host approving its own parked call.
pub const SELF_APPROVAL: &str = "consent.self_approval";
/// A modal-tier call with no independent dialog to answer it (#244).
pub const NO_SURFACE: &str = "consent.no_surface";

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a call an agent loop parked, in a real session.
    fn loop_call(class: ConfirmClass) -> Approval {
        Approval {
            approve: true,
            is_requester: true,
            owns_consent_name: false,
            requester_hosts_a_model: true,
            class,
            brokered: true,
        }
    }

    /// #216, and the reason this module exists. Before it, `Chip` was
    /// approvable by the requester at all times — so the first
    /// write-tier tool handed to the model came with a confirmation the
    /// model's own host could answer.
    #[test]
    fn a_model_host_can_never_approve_its_own_call() {
        for class in [ConfirmClass::Chip, ConfirmClass::Modal] {
            for brokered in [true, false] {
                let v = judge_approval(&Approval {
                    brokered,
                    ..loop_call(class)
                });
                assert_eq!(
                    v.rule(),
                    Some(SELF_APPROVAL),
                    "a model host approved its own {class:?} call (brokered={brokered})"
                );
                assert!(v.is_hard_no(), "{class:?} self-approval must be a hard no");
            }
        }
    }

    /// …and owning the consent name does not help, because independence
    /// is a property of the PAIR. This is #145 in one assertion: the
    /// overlay backend hosted the model AND owned a surface name.
    #[test]
    fn wearing_both_hats_is_not_independence() {
        let v = judge_approval(&Approval {
            owns_consent_name: true,
            ..loop_call(ConfirmClass::Chip)
        });
        assert_eq!(v.rule(), Some(SELF_APPROVAL));
    }

    /// The other half, or the feature does not exist: an independent
    /// surface CAN release the loop's call. A guardrail with no
    /// permitted path is an outage.
    #[test]
    fn the_independent_surface_releases_the_loops_call() {
        for class in [ConfirmClass::Chip, ConfirmClass::Modal] {
            let v = judge_approval(&Approval {
                is_requester: false,
                owns_consent_name: true,
                ..loop_call(class)
            });
            assert_eq!(v, ApprovalVerdict::Allow, "{class:?}");
        }
    }

    /// Withdrawal is not an action, so it is never refused. ADR-0030
    /// §1: guardrails sit between the model and the machine, and a loop
    /// cleaning up after itself is not the machine being touched.
    #[test]
    fn a_model_host_may_still_withdraw_its_own_call() {
        for class in [ConfirmClass::Chip, ConfirmClass::Modal] {
            let v = judge_approval(&Approval {
                approve: false,
                ..loop_call(class)
            });
            assert_eq!(v, ApprovalVerdict::Allow, "{class:?}");
        }
    }

    /// #244: a modal with no dialog is refused, not waved through. The
    /// old code called this state `Absent` and treated it as permission
    /// for the requester to answer itself — on the reference iMac the
    /// consent surface was activatable and never once started, so this
    /// was the live path.
    #[test]
    fn a_modal_with_no_surface_is_refused_even_for_an_ordinary_peer() {
        let v = judge_approval(&Approval {
            requester_hosts_a_model: false,
            ..loop_call(ConfirmClass::Modal)
        });
        assert_eq!(v.rule(), Some(NO_SURFACE));
    }

    /// The behaviour that must NOT change: a person typing `lisa do` in
    /// a terminal answers their own write-tier chip. The CLI is not a
    /// model host (see agentd's `MODEL_HOSTS` and the limit recorded
    /// beside it), and routing this through a modal would train people
    /// to click through the dialog that matters.
    #[test]
    fn an_ordinary_peer_still_answers_its_own_chip() {
        let v = judge_approval(&Approval {
            requester_hosts_a_model: false,
            ..loop_call(ConfirmClass::Chip)
        });
        assert_eq!(v, ApprovalVerdict::Allow);
    }

    /// p2p has one connection, so requester and answerer are the same
    /// peer by construction — for an ordinary peer there is no
    /// separation to enforce and the modal rule stands down. agentd's
    /// own tests are the only thing that reaches this.
    #[test]
    fn an_unbrokered_transport_exempts_only_the_modal_rule() {
        let v = judge_approval(&Approval {
            requester_hosts_a_model: false,
            brokered: false,
            ..loop_call(ConfirmClass::Modal)
        });
        assert_eq!(v, ApprovalVerdict::Allow);
    }

    /// A stranger learns nothing. Not the rule, not the reason, not
    /// whether the call exists (#93, #131).
    #[test]
    fn a_stranger_is_told_nothing_at_all() {
        for approve in [true, false] {
            let v = judge_approval(&Approval {
                approve,
                is_requester: false,
                owns_consent_name: false,
                ..loop_call(ConfirmClass::Chip)
            });
            assert_eq!(v, ApprovalVerdict::NotYours);
            assert_eq!(v.rule(), None, "a refusal to a stranger named a rule");
        }
    }

    /// Both ids are in the catalogue a person can look up, or a Ledger
    /// entry naming one is a dead end.
    #[test]
    fn both_rules_are_in_the_bus_catalogue() {
        for id in [SELF_APPROVAL, NO_SURFACE] {
            assert!(
                crate::BUS_RULES.iter().any(|(r, _)| *r == id),
                "`{id}` is emitted and not catalogued"
            );
        }
        assert!(
            HARD_NO_RULES.contains(&SELF_APPROVAL),
            "self-approval is refused by what it IS, not by the current grant"
        );
    }
}
