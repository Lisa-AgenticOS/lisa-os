//! The private channel between `lisa-consentd` and the dialog it draws
//! with (#289).
//!
//! One JSON object per line, in both directions, over the renderer
//! child's stdin/stdout. Deliberately not D-Bus: the renderer must have
//! **no** way to reach the broker, because the whole point of giving the
//! daemon an executable of its own is that "the peer that owns
//! `dev.lisaos.Consent1`" and "a GJS process" stop being the same
//! sentence. A pipe has one reader and one writer and the parent chose
//! both.
//!
//! Three properties this format has to keep:
//!
//! 1. **The renderer is told a call id and never learns anything it
//!    could act on with it.** It cannot `Confirm`; it has no bus. The id
//!    travels out and comes back so the parent can match an answer to a
//!    dialog, and that is all it is for.
//! 2. **The spec crosses as an opaque string.** agentd's JSON is parsed
//!    by the renderer, which already refuses to render keys it does not
//!    recognise. Re-parsing it here would add a second, differently
//!    permissive reader of attacker-influenced text for no gain.
//! 3. **A refusal report and a confirmation are different message
//!    kinds**, the same way they are different signals on the bus
//!    (#251). A renderer that could mistake one for the other would draw
//!    an Allow button on something with no parked call behind it.

use serde::{Deserialize, Serialize};

/// Parent → renderer. What to put on screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToRenderer {
    /// A parked call needs an answer. `spec` is agentd's
    /// `ConfirmationRequested` payload, verbatim.
    Confirm { call_id: u64, spec: String },
    /// A call was refused outright (#251). There is nothing to answer;
    /// the dialog reports and has no approving control.
    Refusal { call_id: u64, report: String },
}

/// What a person did with a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    /// The pointer hit Allow.
    Allow,
    /// The pointer hit Deny, or closed the window. A closed window is a
    /// denial and never a silent nothing: a dismissed dialog must not
    /// leave a privileged call parked until its TTL, where it looks to
    /// the person like the action is still going to happen.
    Deny,
    /// A refusal report was acknowledged. Carries no authority at all —
    /// there is no parked call behind a refusal, so this can never
    /// become a `Confirm`.
    Dismiss,
}

/// Renderer → parent. The person's answer, and nothing else: the
/// renderer cannot ask the parent to do anything other than report what
/// was clicked on a dialog the parent itself opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FromRenderer {
    pub call_id: u64,
    pub answer: Answer,
}

impl ToRenderer {
    /// One line, newline-terminated. Panics never: these types have no
    /// serialization failure mode.
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).expect("ToRenderer serializes");
        s.push('\n');
        s
    }
}

impl FromRenderer {
    /// The renderer's half of the wire format. Nothing in the daemon
    /// writes this direction — the GJS child does — so it exists here to
    /// pin the format the child must produce, and is compiled only for
    /// the tests that check the two ends agree.
    #[cfg(test)]
    pub fn to_line(self) -> String {
        let mut s = serde_json::to_string(&self).expect("FromRenderer serializes");
        s.push('\n');
        s
    }

    /// Parse one line from the renderer.
    ///
    /// Errors are the caller's problem to ignore *loudly*: a renderer
    /// that has started saying things we do not understand is a renderer
    /// that has been replaced, and the safe response is to answer
    /// nothing — the call stays parked and expires.
    pub fn from_line(line: &str) -> Result<FromRenderer, serde_json::Error> {
        serde_json::from_str(line.trim_end_matches(['\r', '\n']))
    }
}

/// Does this answer release the call?
///
/// A free function rather than a method so the mapping is one place and
/// one line: `Dismiss` is not a denial of anything, because a refusal
/// report has no parked call to deny.
pub fn confirm_for(answer: Answer) -> Option<bool> {
    match answer {
        Answer::Allow => Some(true),
        Answer::Deny => Some(false),
        Answer::Dismiss => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_confirmation_round_trips_with_the_spec_untouched() {
        // The spec is opaque and may contain anything a tool manifest
        // can: quotes, newlines, unicode. It must survive the pipe
        // byte-for-byte, because the renderer's own parser is what
        // decides which keys are safe to draw.
        let spec = "{\"tool\":\"rm\",\"args\":{\"t\":\"a\\nb \\\"quoted\\\" ünïcode\"}}";
        let msg = ToRenderer::Confirm {
            call_id: 41,
            spec: spec.into(),
        };
        let line = msg.to_line();
        assert!(line.ends_with('\n'), "the framing is one line per message");
        assert_eq!(
            line.matches('\n').count(),
            1,
            "an embedded newline in the spec must not become a second frame"
        );
        let back: ToRenderer = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn a_refusal_is_a_different_kind_on_the_wire() {
        // #251: if these were one message with a flag, a renderer bug
        // would draw an Allow button on a refusal. Two variants means
        // the mistake is a parse error, not a button.
        let confirm = ToRenderer::Confirm {
            call_id: 1,
            spec: "{}".into(),
        };
        let refusal = ToRenderer::Refusal {
            call_id: 1,
            report: "{}".into(),
        };
        assert!(confirm.to_line().contains("\"kind\":\"confirm\""));
        assert!(refusal.to_line().contains("\"kind\":\"refusal\""));
        assert_ne!(confirm.to_line(), refusal.to_line());
    }

    #[test]
    fn only_allow_releases_a_call() {
        assert_eq!(confirm_for(Answer::Allow), Some(true));
        assert_eq!(confirm_for(Answer::Deny), Some(false));
        // The one that must never become a `Confirm`: there is no
        // parked call behind a refusal report.
        assert_eq!(confirm_for(Answer::Dismiss), None);
    }

    #[test]
    fn an_answer_the_parent_cannot_read_is_not_an_approval() {
        // Fail closed on the channel too. Anything malformed produces an
        // error, and the caller answers nothing — the call stays parked
        // and expires, which is the safe direction.
        for line in [
            "",
            "not json",
            "{}",
            "{\"call_id\":1}",
            "{\"call_id\":1,\"answer\":\"ALLOW\"}",
            "{\"call_id\":1,\"answer\":\"approve\"}",
            "{\"call_id\":\"1\",\"answer\":\"allow\"}",
        ] {
            assert!(
                FromRenderer::from_line(line).is_err(),
                "`{line}` parsed as an answer"
            );
        }
    }

    #[test]
    fn a_well_formed_answer_round_trips() {
        for answer in [Answer::Allow, Answer::Deny, Answer::Dismiss] {
            let a = FromRenderer { call_id: 7, answer };
            assert_eq!(FromRenderer::from_line(&a.to_line()).unwrap(), a);
        }
        // …and with the trailing newline the reader may or may not
        // strip, because a line reader that keeps it and one that does
        // not are both things we have shipped.
        assert_eq!(
            FromRenderer::from_line("{\"call_id\":7,\"answer\":\"deny\"}\r\n").unwrap(),
            FromRenderer {
                call_id: 7,
                answer: Answer::Deny
            }
        );
    }
}
