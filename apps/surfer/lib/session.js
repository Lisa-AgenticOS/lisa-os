// Session restore — which tabs come back, and the switch that stops
// them coming back at all.
//
// The switch is the part that matters. Reopening a person's tabs is a
// convenience for most people and a hazard for some: the tab you had
// open is visible to anyone who starts the browser, on a shared machine
// or over a shoulder. So it is a setting a person can turn off, it is
// stored per profile like everything else (lib/store.js), and turning it
// off DISCARDS the snapshot rather than merely declining to read it —
// a saved session that survives being switched off is still on disk.
//
// The agent profile never restores: the agent's last tab is not
// something the person asked to see again, and a restored agent tab
// would be page content loading before anybody opened anything.

import {AGENT_PROFILE} from './profiles.js';
import {START_URI} from './startpage.js';

/// The most tabs a restore will open. A session with three hundred tabs
/// is one that must not spend a minute of CPU before the window paints.
export const SESSION_LIMIT = 50;

/// Schemes a restored tab may load.
///
/// An allowlist for the same reason bookmarks have one: a restored tab
/// is a load nobody pressed a key for. `file:` stays — a person's own
/// document, reopened on their own machine.
const RESTORABLE = ['http:', 'https:', 'file:'];

export function restorable(url) {
    const u = String(url ?? '').trim().toLowerCase();
    if (u === '') return false;
    // The start page is where a fresh tab lands anyway; storing it means
    // "restore" and "cold start" differ only in the work done.
    if (u === START_URI.toLowerCase()) return false;
    return RESTORABLE.some(s => u.startsWith(s));
}

/// Is restore on? Default ON — this is what people expect from a
/// browser — and off is anything that is exactly `false`, so a
/// corrupt or half-written settings file does not quietly disable it.
export function restoreEnabled(settings) {
    return settings?.restoreSession !== false;
}

/// The snapshot to write for the tabs currently open.
///
/// Returns `null` when there is nothing worth saving, which the caller
/// must treat as "delete the file": leaving yesterday's snapshot behind
/// because today's is empty is how a closed tab comes back.
export function sessionSnapshot(tabs, {at = 0, profile} = {}) {
    if (profile === AGENT_PROFILE) return null;
    const open = (Array.isArray(tabs) ? tabs : [])
        .filter(t => restorable(t?.url))
        .slice(0, SESSION_LIMIT)
        .map(t => ({url: String(t.url), title: String(t?.title ?? '')}));
    if (open.length === 0) return null;
    // Which tab was in front, clamped into the list that survived the
    // filter — an index pointing past the end selects nothing and
    // leaves the window with no current view.
    const selectedRaw = (Array.isArray(tabs) ? tabs : []).findIndex(t => t?.selected);
    const selectedUrl = selectedRaw >= 0 ? tabs[selectedRaw]?.url : null;
    const selected = Math.max(0, open.findIndex(t => t.url === selectedUrl));
    return {savedAt: typeof at === 'number' ? at : 0, selected, tabs: open};
}

/// The tabs to actually open at start.
///
/// Empty whenever restore is off, the profile is the agent's, or the
/// snapshot is not the shape we wrote — three separate reasons that all
/// mean "open a fresh window", and none of which is an error to report
/// at somebody who just launched a browser.
export function tabsToRestore(snapshot, {settings, profile} = {}) {
    if (profile === AGENT_PROFILE) return [];
    if (!restoreEnabled(settings)) return [];
    const tabs = Array.isArray(snapshot?.tabs) ? snapshot.tabs : [];
    return tabs
        .filter(t => restorable(t?.url))
        .slice(0, SESSION_LIMIT)
        .map(t => ({url: String(t.url), title: String(t?.title ?? '')}));
}

/// Which of the restored tabs to select, clamped.
export function selectedIndex(snapshot, count) {
    const n = typeof count === 'number' && count > 0 ? count : 0;
    if (n === 0) return -1;
    const i = snapshot?.selected;
    if (typeof i !== 'number' || !Number.isFinite(i) || i < 0) return 0;
    return Math.min(Math.floor(i), n - 1);
}
