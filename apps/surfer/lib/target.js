// Which tab a write-tier action acts on (#213, ADR-0033).
//
// # The defect
//
// `click` and `fill` resolved `currentView()` when they ran, and the
// confirmation that authorised them stored no tab at all. So the tab
// could change between "the human approved this" and "this happened":
//
//   - the agent's own `click` opens a popup, `attachTab(popup, true)`
//     selects it, and the next approved `fill` lands in the attacker's
//     field;
//   - a page can do the same to itself with `location.href` on a timer
//     — no gesture, no popup, no click needed.
//
// This is the ADR-0033 shape: a later call acting on state that nothing
// pinned and nothing checked.
//
// # The rule
//
// A write says which page it means, by URL. That URL is an ARGUMENT, so
// it is what the consent dialog shows the human — approving an action
// whose target you cannot see is not consent — and it is checked here
// against the tabs that actually exist at the moment the action runs.
// If the page it describes is not open any more, the action is refused
// rather than redirected at whatever is in front of it.
//
// Pure: tabs come in as `{url, selected}` in tab order, and the answer
// is an index. No gi:// import, so every rule here is testable off a
// display.

/// Two URLs naming the same page. Exact, except for one trailing slash:
/// the engine reports `https://example.org/` for what a model echoes
/// back as `https://example.org`. Dropping a single trailing slash
/// cannot merge two different origins or two different paths, and
/// nothing else is normalised — a query or a fragment that differs is a
/// different page, and being strict here costs a retry, while being
/// loose costs the guarantee.
function sameTarget(a, b) {
    const trim = (u) => String(u ?? '').replace(/\/$/, '');
    return trim(a) === trim(b);
}

/// Pin the tab a write-tier action names. Returns its index, or throws
/// with a reason the caller can hand back to the agent verbatim.
export function pinTarget(tabs, {url} = {}) {
    const wanted = String(url ?? '').trim();
    if (wanted === '') {
        throw new Error(
            'this action must say which page it acts on: pass url with the ' +
            'address read_page returned');
    }
    const open = Array.isArray(tabs) ? tabs : [];
    const matches = [];
    for (let i = 0; i < open.length; i++) {
        if (sameTarget(open[i]?.url, wanted)) matches.push(i);
    }
    if (matches.length === 0) {
        throw new Error(
            `no open tab is at ${JSON.stringify(wanted)} — the page moved or ` +
            'the tab was closed, so this action was not performed');
    }
    // One tab, or the one the user is looking at. Anything else is a
    // guess, and a guess is not what was confirmed.
    const selected = matches.find(i => open[i]?.selected);
    if (selected !== undefined) return selected;
    if (matches.length === 1) return matches[0];
    throw new Error(
        `${matches.length} tabs are at ${JSON.stringify(wanted)} — select the ` +
        'one you mean and try again');
}
