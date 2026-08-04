// The Spotlight chord's decision (PLAN §5.7.2, issue #255).
//
// Kept out of extension.js and free of GNOME imports so it can be
// tested on a dev host: the shell half is three lines of St/Meta that
// only a live session can exercise, and this is the part that can be
// wrong.

/// What Super+Space should do, given the overview's current state.
///
/// Returns 'open' (show the overview and put the caret in the search
/// entry) or 'dismiss' (close it). Anything missing reads as 'open',
/// because the failure a person notices is a summon key that does
/// nothing — never a key that opened a search box they can Escape out
/// of.
export function summonAction(state) {
    const overviewVisible = state?.overviewVisible === true;
    const searchActive = state?.searchActive === true;
    return overviewVisible && searchActive ? 'dismiss' : 'open';
}
