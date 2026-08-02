// What a link in a message is allowed to do. Pure, so the rules are
// testable without a display — the same reason apps/surfer/lib/url.js
// is a separate module, and the same rules, because a mail body is
// exactly as untrusted as a web page.
//
// Found in review (2026-08-02): the reading pane had neither rule.
//
//   1. `data:` was allowed to NAVIGATE. It is legitimately needed for
//      inline image bytes, but that is a sub-resource decision
//      (PolicyDecisionType.RESPONSE); allowing it for
//      NAVIGATION_ACTION means a clicked `data:text/html,…` link
//      replaces the reading pane with attacker-authored markup. With
//      JavaScript disabled that is a convincing spoof rather than code
//      execution — a fake login form drawn inside the mail client is
//      still the whole of phishing.
//
//   2. Clicked links went straight to `Gtk.show_uri` with no scheme
//      check, so `file:///…`, `smb://…` or anything else the desktop
//      has a handler for could be opened by a sender.

/// Schemes a person can be sent to, by clicking, in an external app.
/// Deliberately short: this is a list of things it is safe for a
/// stranger to put in front of you.
const OPENABLE = ['http:', 'https:', 'mailto:'];

/// What should happen when the reading pane is asked to navigate.
///
/// - `in-place`  — the message document loading itself.
/// - `external`  — hand to the desktop; the user sees where they go.
/// - `refuse`    — neither, with a reason worth showing.
export function linkAction(rawUri) {
    const uri = String(rawUri ?? '').trim();
    // The message's own load. `load_html` navigates to about:blank, and
    // an empty URI is the same event seen from a different angle.
    if (uri === '' || uri.toLowerCase().startsWith('about:'))
        return {action: 'in-place'};

    const lower = uri.toLowerCase();
    for (const scheme of OPENABLE) {
        if (lower.startsWith(scheme))
            return {action: 'external', uri};
    }
    // Everything else, named rather than silently dropped: cid: and
    // data: belong to sub-resources, javascript: executes, file: and
    // smb: reach the machine and the network on a sender's say-so.
    return {
        action: 'refuse',
        uri,
        reason: `links of this kind are not opened from mail (${lower.split(':')[0]}:)`,
    };
}
