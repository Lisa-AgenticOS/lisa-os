// Where one profile's browsing data lives on disk (#146 follow-up:
// downloads, history, bookmarks, session).
//
// One module, because the property being defended is one property:
// **a profile's file never resolves inside another profile's
// directory.** downloads.js, history.js, bookmarks.js and session.js are
// pure reducers over arrays and have no idea where they are written;
// this is the only code that turns a profile name into a path, so it is
// the only place that could leak one profile's browsing into another's.
//
// The agent profile matters most here. profiles.js already confines
// agent-driven browsing to its own `NetworkSession` (#181, #259); that
// buys nothing if the agent's history is appended to the person's file.
//
// No gi:// import: the app does the file I/O, this decides the path and
// what a corrupt file means.

import {dataDirFor} from './profiles.js';

/// The files a profile may have.
///
/// An ALLOWLIST rather than a sanitiser. Every caller is our own code,
/// so a name that is not one of these is a bug — and a bug that reached
/// a path would be a traversal. Nothing here is ever derived from a
/// page, a URL or a tool argument.
export const STORE_FILES = Object.freeze({
    history: 'history.json',
    bookmarks: 'bookmarks.json',
    downloads: 'downloads.json',
    session: 'session.json',
    settings: 'settings.json',
});

/// The absolute path of one profile's store file, or `null` when either
/// the profile or the kind is not one we recognise.
///
/// `null` is not an error to paper over: the caller must skip the write
/// entirely rather than fall back to some other profile's file, which is
/// precisely the leak this module exists to prevent.
export function profileStorePath(profile, base, kind) {
    if (typeof kind !== 'string' ||
        !Object.prototype.hasOwnProperty.call(STORE_FILES, kind))
        return null;
    if (typeof base !== 'string' || base === '')
        return null;
    const dir = dataDirFor(profile, base);
    if (!dir) return null;
    return `${dir}/${STORE_FILES[kind]}`;
}

/// A stored file's text → the list it holds.
///
/// Anything that is not a JSON object with an array under `key` comes
/// back as `[]`. A browser whose history file was truncated by a power
/// cut must open, not crash: losing a history is bad and being unable
/// to start is worse.
export function decodeStore(text, key) {
    if (typeof text !== 'string' || text.trim() === '') return [];
    let parsed = null;
    try {
        parsed = JSON.parse(text);
    } catch {
        return [];
    }
    if (!parsed || typeof parsed !== 'object') return [];
    const items = parsed[key];
    return Array.isArray(items) ? items : [];
}

/// The text to write for a list. Versioned, because the day the shape
/// changes an unversioned file is indistinguishable from a corrupt one.
export function encodeStore(key, items) {
    return JSON.stringify({
        surfer_store: 1,
        [key]: Array.isArray(items) ? items : [],
    });
}

/// A stored file's text → the settings object it holds.
///
/// Same tolerance as `decodeStore`, different shape: settings are a map,
/// not a list, and a missing key must read as "not set" so every
/// consumer applies its own default rather than inheriting `{}`'s.
export function decodeSettings(text) {
    if (typeof text !== 'string' || text.trim() === '') return {};
    let parsed = null;
    try {
        parsed = JSON.parse(text);
    } catch {
        return {};
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {};
    return parsed;
}

/// The text to write for settings.
export function encodeSettings(settings) {
    const s = settings && typeof settings === 'object' && !Array.isArray(settings)
        ? settings : {};
    return JSON.stringify({surfer_store: 1, ...s});
}
