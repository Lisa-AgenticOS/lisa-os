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
//
// Every record carries a `mode` — which navrail surface the conversation
// belongs to (lib/modes.js), so the sidebar can show only the active
// mode's chats. It sits between `updated_ts` and `turns` on both sides
// of the format; a record stored before modes existed reads as `chat`,
// and an unknown stored mode collapses to `chat` too (`wireMode`), so a
// renamed mode hides no one's conversation.

import {normalizeTurns, deserializeConversation} from './model.js';
import {DEFAULT_MODE, wireMode} from './modes.js';

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
 * @param {string} [mode]  the navrail mode this conversation belongs to
 * @returns {{id: string, title: string, created_ts: number,
 *            updated_ts: number, mode: string, turns: object[]}}
 */
export function newSession(title = UNTITLED, now = Date.now(), mode = DEFAULT_MODE) {
    return {
        id: newSessionId(now),
        title,
        created_ts: now,
        updated_ts: now,
        mode: wireMode(mode),
        turns: [],
    };
}

/**
 * A session's listing entry — what the index stores. `mode` is
 * validated on the way through: missing (a pre-mode record) and unknown
 * both read as the default, so old and future data land in Chat rather
 * than nowhere.
 * @param {object} session
 * @returns {{id: string, title: string, created_ts: number,
 *            updated_ts: number, mode: string}}
 */
export function sessionInfo(session) {
    return {
        id: String(session?.id ?? ''),
        title: typeof session?.title === 'string' && session.title !== ''
            ? session.title : UNTITLED,
        created_ts: Number(session?.created_ts ?? 0),
        updated_ts: Number(session?.updated_ts ?? 0),
        mode: wireMode(session?.mode),
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
        mode: info.mode,
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
 * The index entries belonging to navrail mode `mode` — what the sidebar
 * shows while that mode is active, so a coding session and a research
 * thread do not braid. Both sides go through `wireMode`, so a pre-mode
 * entry (no field) and an unknown mode are both Chat's — every stored
 * conversation is reachable from exactly one rail button.
 * @param {object[]} index
 * @param {string} mode
 * @returns {object[]}  a new array
 */
export function indexForMode(index, mode) {
    const want = wireMode(mode);
    return (index ?? []).filter(e => wireMode(e?.mode) === want);
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
 * The listing rebuilt out of the stored records themselves.
 *
 * Recovery, and it has work to do (#228). `_memoryGet` mapped every
 * failure to `''` — including `AccessDenied` — so the window read an
 * empty index, and the next completed turn wrote `upsertIndex([], …)`:
 * an index of exactly ONE conversation, replacing the real one. The
 * other conversations were never deleted, because Context1 has no
 * per-key delete; they are still at `session/<id>`, addressable and
 * unlisted. This is how they come back.
 * @param {?Object<string, string>} map  the namespace, key → value
 * @returns {object[]}  index entries, most recently active first
 */
export function indexFromRecords(map) {
    const rows = [];
    for (const [key, value] of Object.entries(map ?? {})) {
        if (!key.startsWith(SESSION_KEY_PREFIX))
            continue;
        // parseSession already refuses a tombstone and a corrupt record.
        const record = parseSession(value);
        if (record)
            rows.push(sessionInfo(record));
    }
    return rows.sort((a, b) => b.updated_ts - a.updated_ts);
}

/**
 * `first` wins for any id it has; everything only `second` knows about
 * is added. Ordered by activity, like every other listing.
 * @param {object[]} first
 * @param {object[]} second
 * @returns {object[]}  a new array
 */
export function mergeIndex(first, second) {
    const kept = (first ?? []).map(sessionInfo);
    const seen = new Set(kept.map(e => e.id));
    const added = (second ?? []).map(sessionInfo).filter(e => !seen.has(e.id));
    return [...kept, ...added].sort((a, b) => b.updated_ts - a.updated_ts);
}

/**
 * What a restore learned about the app-memory namespace.
 *
 * The whole point is the `indexKnown` bit, and it is #228's second and
 * more dangerous half — the #210 shape. Rewriting `sessions` REPLACES
 * the list of every conversation the person has, so it may only ever
 * happen from a read that authoritatively said what is stored. A call
 * that FAILED is not that. It used to be: `catch { return ''; }`, and
 * `''` parses as an empty index, so one AccessDenied cost the person
 * every conversation in their list.
 *
 * An empty namespace is a different answer and a fine one — first run,
 * nothing stored, write away.
 * @param {{ok: boolean, map?: Object<string,string>, error?: string}} read
 * @param {object[]} [open]  index entries the window already holds
 * @returns {{indexKnown: boolean, sessions: object[], legacy: string,
 *            note: ?string}}
 */
export function restorePlan(read, open = []) {
    if (!read?.ok) {
        return {
            indexKnown: false,
            sessions: (open ?? []).map(sessionInfo),
            legacy: '',
            note: 'Stored conversations could not be read ' +
                `(${read?.error || 'the context daemon did not answer'}). ` +
                'Nothing is saved this session — and nothing already ' +
                'saved has been changed.',
        };
    }
    const map = read.map ?? {};
    const stored = parseSessionIndex(map[INDEX_KEY]);
    const recovered = indexFromRecords(map);
    const orphans = recovered.filter(e => !stored.some(s => s.id === e.id));
    return {
        indexKnown: true,
        // What is open beats both: the person is looking at it.
        sessions: mergeIndex(mergeIndex(stored, recovered), open),
        legacy: typeof map[LEGACY_CONVERSATION_KEY] === 'string'
            ? map[LEGACY_CONVERSATION_KEY] : '',
        note: orphans.length === 0 ? null
            : `Recovered ${orphans.length} conversation` +
              `${orphans.length === 1 ? '' : 's'} that had dropped off the list.`,
    };
}

/**
 * What to do with a Spotlight hand-off (`ask(prompt)`).
 *
 * `_send` returned early whenever a run was in flight, so a hand-off
 * arriving mid-stream overwrote the composer and then went nowhere: no
 * new session, no turn, no error, nothing in the log (#233). The
 * overlay had already closed, so the person's question was simply gone.
 *
 * Queued rather than refused, and never merged into the running
 * conversation: the reply that is streaming belongs to the conversation
 * that asked for it, and what was typed into the overlay's empty box is
 * a new thought (#210). It starts its own conversation when the current
 * run ends — and it says so, because a queue nobody can see is the same
 * silence in a nicer shape.
 * @param {?string} prompt
 * @param {{busy?: boolean, hasTurns?: boolean}} [state]
 * @returns {{action: 'send'|'queue'|'ignore', prompt: string,
 *            newSession: boolean, note: ?string}}
 */
export function handoffPlan(prompt, state = {}) {
    const text = String(prompt ?? '').trim();
    if (text === '')
        return {action: 'ignore', prompt: '', newSession: false, note: null};
    if (state.busy) {
        return {
            action: 'queue',
            prompt: text,
            newSession: true,
            note: `“${text}” will start a new conversation when this reply ` +
                'finishes.',
        };
    }
    // A blank window is already the fresh conversation a hand-off wants.
    return {
        action: 'send',
        prompt: text,
        newSession: !!state.hasTurns,
        note: null,
    };
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
