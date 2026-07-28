// Tab state, with no GTK in it (ADR-0037, issue #146).
//
// Pure because tab bugs are the ones users notice: close the third of
// five tabs and the wrong one gets focus, close the last one and the
// window is empty but alive, reorder and the active index points at a
// stranger. Every one of those is arithmetic, and arithmetic belongs
// somewhere it can be tested without a display.
//
// The model is a list plus an active index. Ids are opaque and never
// reused, so a stale reference from the UI cannot resolve to a tab that
// took its place.

let nextId = 1;

export function newTabs() {
    return {tabs: [], active: null};
}

/// Open a tab, optionally after a given one (a link opened from tab 2
/// belongs beside tab 2, not at the far end of the strip).
export function open(state, {url = 'about:blank', title = '', after = null, focus = true} = {}) {
    const tab = {id: nextId++, url, title, loading: false};
    const at = after === null ? state.tabs.length : indexOf(state, after) + 1;
    const tabs = [...state.tabs.slice(0, at), tab, ...state.tabs.slice(at)];
    return {
        tabs,
        active: focus || state.active === null ? tab.id : state.active,
    };
}

/// Close a tab and choose what gets focus.
///
/// Focus goes to the RIGHT neighbour, falling back to the left when the
/// closed tab was last. That is what every browser does and what hands
/// expect: closing a run of tabs left-to-right should not walk you
/// backwards through the ones you already dealt with.
export function close(state, id) {
    const i = indexOf(state, id);
    if (i < 0)
        return state;
    const tabs = state.tabs.filter(t => t.id !== id);
    if (tabs.length === 0)
        return {tabs, active: null};
    let active = state.active;
    if (state.active === id)
        active = (tabs[i] ?? tabs[tabs.length - 1]).id;
    return {tabs, active};
}

export function activate(state, id) {
    return indexOf(state, id) < 0 ? state : {...state, active: id};
}

/// Move a tab to `to`, clamped. Out-of-range is clamped rather than
/// refused: a drag past the end of the strip is a real gesture with an
/// obvious meaning.
export function move(state, id, to) {
    const from = indexOf(state, id);
    if (from < 0)
        return state;
    const target = Math.max(0, Math.min(to, state.tabs.length - 1));
    if (target === from)
        return state;
    const tabs = [...state.tabs];
    const [tab] = tabs.splice(from, 1);
    tabs.splice(target, 0, tab);
    return {...state, tabs};
}

/// Update one tab's fields, leaving the rest alone.
export function update(state, id, fields) {
    return {
        ...state,
        tabs: state.tabs.map(t => (t.id === id ? {...t, ...fields} : t)),
    };
}

export function activeTab(state) {
    return state.tabs.find(t => t.id === state.active) ?? null;
}

function indexOf(state, id) {
    return state.tabs.findIndex(t => t.id === id);
}

/// Test seam: ids are module-global and monotonic, so a test that
/// asserts on them needs a way back to a known start.
export function __resetIds(to = 1) {
    nextId = to;
}
