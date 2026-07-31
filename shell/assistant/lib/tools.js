// The Agent Bus, as the Assistant needs to see it.
//
// Pure: what to ask for, and what to do with the answer. The D-Bus calls
// live in the window; everything that can be got wrong is here, where a
// test can reach it.
//
// # Read-tier only, deliberately
//
// This window can ask what is in your mail and your notes. It cannot
// send, delete, or file anything. That is not caution for its own sake —
// a write-tier call parks for confirmation, and answering that
// confirmation from inside the model host is the hole #145 was opened to
// close. The consent surface is now a separate process, so write tier is
// *defensible*; it is still a second thing to get right and it is not
// this change.
//
// The filter is the daemon's OWN tier, not a list of names kept here.
//
// The first version of this file hardcoded six names, three of which did
// not exist — `page_text`, `list_tabs` and `read_note` were invented,
// while the device actually offers `read_page`, `get_selection`,
// `screenshot` and `list_notes`. A hand-maintained list is wrong the day
// an app ships a tool, and wrong silently: the model is simply never
// told the tool exists.
//
// `ListTools()` reports `tier` per tool, and agentd enforces it. Reading
// that field means this surface stays correct as apps come and go, and
// means the Assistant and the daemon can never disagree about what
// read-tier is.
export const READ_TIER = 'read';

/// Keep only the tools this surface is willing to offer.
///
/// `catalog` is agentd's `ListTools()` payload. An entry with no `tier`
/// is dropped rather than assumed read — the failure of assuming would
/// be offering a destructive tool, and the failure of dropping is a
/// missing capability somebody notices and reports.
export function offerable(catalog) {
    let tools;
    try {
        tools = typeof catalog === 'string' ? JSON.parse(catalog) : catalog;
    } catch {
        return [];
    }
    if (!Array.isArray(tools))
        return [];
    return tools.filter(
        (t) => t && typeof t.name === 'string' && t.tier === READ_TIER);
}

/// The options for `RequestCall`.
///
/// `provenance: ['user']` is a claim about where this request came from,
/// and agentd now verifies it against peer credentials (#55) — the
/// Assistant is one of Lisa's own programs, so the claim holds. It is
/// stated rather than omitted because an empty chain is "unknown" and
/// escalates, which would ask the person to confirm reading their own
/// mail.
export function callOptions() {
    return {actor: 'user', provenance: ['user']};
}

/// What the window should do with a disposition.
///
/// The dispositions that matter here are the two this surface must
/// NEVER handle silently: a parked confirmation is somebody else's to
/// answer, and a denial is a refusal to report rather than retry.
export function interpret(disposition, detailJson) {
    let detail = {};
    try {
        detail = JSON.parse(detailJson ?? '{}');
    } catch {
        detail = {};
    }
    switch (disposition) {
    case 'executed':
        return {kind: 'result', text: detail.result ?? ''};
    case 'failed':
        return {kind: 'error', text: detail.error ?? 'the tool failed'};
    case 'denied':
        // A refusal is an answer. Retrying it, or rephrasing it into
        // something that gets through, is the behaviour a guardrail
        // exists to prevent (ADR-0030).
        return {kind: 'denied', text: detail.reason ?? 'refused'};
    case 'confirm-chip':
    case 'confirm-modal':
        // The Assistant does not answer this and must not: it is the
        // peer that asked. The consent surface owns the dialog (#145).
        // Reporting it back to the model is what keeps this window
        // honest about what it did and did not do.
        return {
            kind: 'parked',
            text: 'That needs your confirmation — the consent dialog has it.',
        };
    default:
        return {kind: 'error', text: `unexpected disposition ${disposition}`};
    }
}

/// A tool result, trimmed to something a conversation can carry.
///
/// A mail search over four thousand messages can return more text than
/// the model's whole context window. Truncating here, visibly, beats
/// letting the transcript trimmer silently drop the *question* to make
/// room for the answer.
export function forTranscript(text, max = 4000) {
    const s = String(text ?? '');
    if (s.length <= max)
        return s;
    return `${s.slice(0, max)}\n…[${s.length - max} more characters not shown]`;
}
