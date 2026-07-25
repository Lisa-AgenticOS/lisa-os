// Lisa Assistant — conversation sessions (issue #25; PLAN §5.7.1).
//
// The key layout mirrors harness-core's SessionStore
// (libs/harness-core/src/session.rs, ADR-0013) exactly: one JSON record
// per session under `session/<id>` plus one index under `sessions`, all
// inside the app's own `dev.lisaos.Context1` memory namespace. Same
// keys, same field order, same turn shape — so a session written here
// loads in Rust and vice versa, with no new daemon surface to install.
//
// No GNOME imports (see lib/model.js): the window owns the widgets and
// the D-Bus calls, this module owns the transforms, so `just shell-test`
// can run them on any dev host.
//
// Nothing read back from the store is trusted. Context1 has no per-key
// delete — only a namespace-wide MemoryWipe — so a removed session is
// tombstoned with the empty string, and readers treat empty exactly as
// missing (KvStore::remove, libs/harness-core/src/store.rs).

import {normalizeTurns, deserializeConversation} from './model.js';

/// The index key: a JSON array of session info entries.
export const INDEX_KEY = 'sessions';

/// Per-session key prefix; the full key is `session/<id>`.
export const SESSION_KEY_PREFIX = 'session/';

/// Where the Assistant stored its one conversation before sessions
/// existed. Read once at first launch, then tombstoned.
export const LEGACY_CONVERSATION_KEY = 'conversation';

/// A session with no user turn to take a title from.
export const UNTITLED = 'New conversation';

/** The app-memory key holding session `id`. */
export function sessionKey(id) {
    return `${SESSION_KEY_PREFIX}${id}`;
}

let idCounter = 0;

/**
 * A unique-enough session id. Opaque to both sides — only the `s-`
 * prefix is shared with the Rust generator; the entropy differs (no pid
 * in GJS) and nothing parses these.
 * @param {number} [now]  unix milliseconds
 * @returns {string}
 */
export function newSessionId(now = Date.now()) {
    idCounter = (idCounter + 1) & 0xffff;
    const rand = Math.floor(Math.random() * 0x1000000)
        .toString(16).padStart(6, '0');
    return `s-${Math.floor(now).toString(16)}-${idCounter.toString(16)}-${rand}`;
}

/**
 * A fresh, empty session record. Not written anywhere: the window
 * persists a session on its first completed turn, so abandoning a new
 * conversation leaves nothing behind.
 * @param {string} [title]
 * @param {number} [now]  unix milliseconds
 * @returns {{id: string, title: string, created_ts: number,
 *            updated_ts: number, turns: object[]}}
 */
export function newSession(title = UNTITLED, now = Date.now()) {
    return {
        id: newSessionId(now),
        title,
        created_ts: now,
        updated_ts: now,
        turns: [],
    };
}

/**
 * A session's listing entry — what the index stores.
 * @param {object} session
 * @returns {{id: string, title: string, created_ts: number, updated_ts: number}}
 */
export function sessionInfo(session) {
    return {
        id: String(session?.id ?? ''),
        title: typeof session?.title === 'string' && session.title !== ''
            ? session.title : UNTITLED,
        created_ts: Number(session?.created_ts ?? 0),
        updated_ts: Number(session?.updated_ts ?? 0),
    };
}

/** An index entry is only usable with an id and two real timestamps. */
function validInfo(e) {
    return e !== null && typeof e === 'object' &&
        typeof e.id === 'string' && e.id !== '' &&
        typeof e.title === 'string' &&
        Number.isFinite(e.created_ts) && Number.isFinite(e.updated_ts);
}

/**
 * The stored index, most recently active first. A missing, tombstoned,
 * or unparseable value reads as empty; junk entries inside a well-formed
 * array are dropped rather than trusted, so a corrupt namespace can
 * never break startup.
 * @param {?string} json
 * @returns {object[]}
 */
export function parseSessionIndex(json) {
    let parsed;
    try {
        parsed = JSON.parse(json ?? '');
    } catch {
        return [];
    }
    if (!Array.isArray(parsed))
        return [];
    // Stable sort: same-millisecond ties keep array order, which is the
    // insertion order the Rust store relies on for exact activity order.
    return parsed.filter(validInfo).map(sessionInfo)
        .sort((a, b) => b.updated_ts - a.updated_ts);
}

/**
 * @param {object[]} index
 * @returns {string}  the `sessions` value
 */
export function serializeSessionIndex(index) {
    return JSON.stringify((index ?? []).map(sessionInfo));
}

/**
 * One session record. Key order matches the Rust struct so a rewrite
 * from either side leaves the same bytes.
 * @param {object} session  info fields + turns
 * @returns {string}  the `session/<id>` value
 */
export function serializeSession(session) {
    const info = sessionInfo(session);
    return JSON.stringify({
        id: info.id,
        title: info.title,
        created_ts: info.created_ts,
        updated_ts: info.updated_ts,
        turns: normalizeTurns(session?.turns),
    });
}

/**
 * Parse a stored session record. Returns null for a missing,
 * tombstoned, or corrupt value — the window says so rather than
 * silently showing an empty conversation, because losing a conversation
 * quietly is worse than an error (session.rs, Error::Corrupt).
 * @param {?string} json
 * @returns {?object}
 */
export function parseSession(json) {
    let parsed;
    try {
        parsed = JSON.parse(json ?? '');
    } catch {
        return null;
    }
    if (!validInfo(parsed))
        return null;
    return {
        ...sessionInfo(parsed),
        turns: Array.isArray(parsed.turns) ? normalizeTurns(parsed.turns) : [],
    };
}

/**
 * The session record to write after `turns` changed: the title follows
 * the first user turn (it never moves, so this is stable across
 * rewrites) and activity bumps `updated_ts`.
 * @param {object} info  the open session's info fields
 * @param {object[]} turns
 * @param {number} [now]  unix milliseconds
 * @returns {object}
 */
export function sessionWithTurns(info, turns, now = Date.now()) {
    const normalized = normalizeTurns(turns);
    return {
        ...sessionInfo(info),
        title: titleFromTurns(normalized, info?.title),
        updated_ts: now,
        turns: normalized,
    };
}

/**
 * Move `info` to the front of the index, replacing any earlier entry
 * for the same session. Array position breaks same-millisecond ties, so
 * the listing's activity order is exact even within one ms.
 * @param {object[]} index
 * @param {object} info
 * @returns {object[]}  a new array
 */
export function upsertIndex(index, info) {
    const entry = sessionInfo(info);
    return [entry, ...(index ?? []).filter(e => e.id !== entry.id)];
}

/**
 * @param {object[]} index
 * @param {string} id
 * @returns {object[]}  a new array
 */
export function removeFromIndex(index, id) {
    return (index ?? []).filter(e => e.id !== id);
}

/**
 * The rows the conversation list shows: the stored index, with the open
 * session pinned to the front while it is still unwritten. Merely
 * opening an older conversation must not reorder the list — only
 * activity does that (upsertIndex).
 * @param {object[]} index
 * @param {?object} current  the open session's info
 * @returns {object[]}
 */
export function displayIndex(index, current) {
    const rows = index ?? [];
    if (!current?.id || rows.some(e => e.id === current.id))
        return rows;
    return [sessionInfo(current), ...rows];
}

/**
 * A conversation's list title: the first user turn, whitespace
 * collapsed and clipped on a word boundary. Falls back to the title the
 * session already has, so a conversation that only ever got assistant
 * text keeps its name.
 * @param {{role: string, text: string}[]} turns
 * @param {string} [fallback]
 * @param {number} [max]  clip length in characters
 * @returns {string}
 */
export function titleFromTurns(turns, fallback = UNTITLED, max = 42) {
    const base = typeof fallback === 'string' && fallback !== ''
        ? fallback : UNTITLED;
    const first = (turns ?? []).find(t =>
        t && t.role === 'user' && typeof t.text === 'string' &&
        t.text.trim() !== '');
    if (!first)
        return base;
    const text = first.text.replace(/\s+/g, ' ').trim();
    if (text.length <= max)
        return text;
    const cut = text.slice(0, max);
    const space = cut.lastIndexOf(' ');
    return `${(space > max * 0.6 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

/**
 * The pre-sessions single conversation folded into a first session, so
 * an upgrading user finds their history where it always was. Null when
 * there is nothing to migrate (no daemon, no key, empty or corrupt
 * value) — the caller then just starts a blank conversation.
 * @param {?string} json  the stored `conversation` value
 * @param {number} [now]  unix milliseconds
 * @returns {?object}  a session record
 */
export function migrateLegacyConversation(json, now = Date.now()) {
    const turns = deserializeConversation(json ?? '');
    if (turns.length === 0)
        return null;
    const session = newSession(UNTITLED, now);
    session.title = titleFromTurns(turns);
    session.turns = turns;
    return session;
}

/**
 * A conversation's "last active" subtitle. Deliberately locale-free:
 * the list is scanned, not read, and a fixed form keeps the sidebar
 * width predictable.
 * @param {number} ts  unix milliseconds
 * @param {number} [now]
 * @returns {string}
 */
export function formatSessionTime(ts, now = Date.now()) {
    if (!Number.isFinite(ts) || ts <= 0)
        return '';
    const secs = Math.max(0, Math.round((now - ts) / 1000));
    if (secs < 60)
        return 'just now';
    const mins = Math.floor(secs / 60);
    if (mins < 60)
        return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24)
        return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    if (days < 7)
        return `${days}d ago`;
    return new Date(ts).toISOString().slice(0, 10);
}
