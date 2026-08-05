// Did an agent cause this? (#146 follow-up, #260)
//
// One question, asked by more than one feature, so it lives in one
// place rather than being re-derived per feature.
//
// The shape came from downloads (f686c9b): Surfer has no `download`
// tool, but `navigate` and `click` are enough — an http address that
// answers `Content-Disposition: attachment` writes a file without any
// tool called `download`. So the window STAMPS a view whenever an
// agent-driven action touches it, the stamp is inherited by popups, and
// anything consequential that starts inside the stamp's lifetime is
// refused.
//
// #260 needs exactly the same test for a different consequence: a
// credential must never be saved or filled because an agent submitted a
// form. Writing a second timer with its own window and its own skew
// handling would be two mechanisms to keep correct; this is one.
//
// No gi:// import: the clock is the window's, the rule is here.

/// How long after an agent touched a view that agent is still the cause.
///
/// Generous on purpose: a redirect chain to an attachment, or a
/// scripted form submit after a `fill`, takes longer than a click does.
export const AGENT_ACTION_WINDOW_MS = 5000;

/// Did an agent cause this?
///
/// Fails CLOSED on everything ambiguous — an unreadable `now`, a clock
/// that went backwards. `now` before the stamp is not a reason to allow
/// a write to disk, and it is not a reason to hand over a password
/// either.
///
/// The one thing that is NOT ambiguous is the absence of a stamp: a view
/// nothing has touched has `agentTouchedAt === undefined`, and treating
/// that as agent-driven would refuse every ordinary download and every
/// ordinary autofill. So the missing-stamp case answers `false`, and it
/// is the only case that does.
export function agentDriven({agentTouchedAt, now, windowMs = AGENT_ACTION_WINDOW_MS} = {}) {
    if (typeof agentTouchedAt !== 'number' || !Number.isFinite(agentTouchedAt))
        return false;
    if (agentTouchedAt <= 0) return false;
    if (typeof now !== 'number' || !Number.isFinite(now)) return true;
    if (now < agentTouchedAt) return true;   // clock skew: fail closed
    return now - agentTouchedAt < windowMs;
}
