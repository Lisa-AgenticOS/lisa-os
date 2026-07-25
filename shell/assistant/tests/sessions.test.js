// Unit tests for the Assistant's conversation sessions (issue #25):
// the key layout it shares with harness-core's SessionStore, index
// ordering, titles, the legacy-conversation migration, and the
// tolerance that keeps a corrupt namespace from breaking startup.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    INDEX_KEY, SESSION_KEY_PREFIX, LEGACY_CONVERSATION_KEY, UNTITLED,
    sessionKey, newSessionId, newSession, sessionInfo,
    parseSessionIndex, serializeSessionIndex, parseSession, serializeSession,
    sessionWithTurns, upsertIndex, removeFromIndex, displayIndex,
    titleFromTurns, migrateLegacyConversation, formatSessionTime,
} from '../lib/sessions.js';

// ---- key layout (must match libs/harness-core/src/session.rs) -------

test('keys match the Rust SessionStore layout', () => {
    assertEq(INDEX_KEY, 'sessions');
    assertEq(SESSION_KEY_PREFIX, 'session/');
    assertEq(LEGACY_CONVERSATION_KEY, 'conversation');
    assertEq(sessionKey('s-42'), 'session/s-42');
});

test('session ids are prefixed and unique', () => {
    const ids = new Set();
    for (let i = 0; i < 100; i++) {
        const id = newSessionId(1700000000000);
        assert(id.startsWith('s-'), `${id} carries the s- prefix`);
        ids.add(id);
    }
    assertEq(ids.size, 100, 'ids minted in the same millisecond differ');
});

test('a new session starts empty and untitled', () => {
    const s = newSession(undefined, 1000);
    assertEq(s.title, UNTITLED);
    assertEq(s.turns, []);
    assertEq(s.created_ts, 1000);
    assertEq(s.updated_ts, 1000);
});

// ---- the record on the wire ----------------------------------------

test('serializeSession writes the SessionStore record shape', () => {
    const session = {
        id: 's-1', title: 'theme this app',
        created_ts: 10, updated_ts: 20,
        turns: [
            {role: 'user', text: 'hi', widget: {}},
            {role: 'assistant', text: 'hello', model: 'qwen3'},
            {role: 'assistant', text: ''},   // in-flight — dropped
        ],
    };
    assertEq(serializeSession(session), JSON.stringify({
        id: 's-1', title: 'theme this app', created_ts: 10, updated_ts: 20,
        turns: [
            {role: 'user', text: 'hi', model: null},
            {role: 'assistant', text: 'hello', model: 'qwen3'},
        ],
    }), 'field order and turn shape are byte-compatible with Rust');
});

test('session records round-trip', () => {
    const session = {
        id: 's-1', title: 't', created_ts: 10, updated_ts: 20,
        turns: [{role: 'user', text: 'q', model: null}],
    };
    assertEq(parseSession(serializeSession(session)), session);
});

test('parseSession refuses missing, tombstoned, and corrupt values', () => {
    assertEq(parseSession(null), null);
    assertEq(parseSession(''), null, 'a tombstone is not a conversation');
    assertEq(parseSession('{broken'), null);
    assertEq(parseSession('[]'), null);
    assertEq(parseSession('{"id":"s-1","title":"t"}'), null, 'no timestamps');
});

test('parseSession drops junk turns instead of trusting them', () => {
    const raw = JSON.stringify({
        id: 's-1', title: 't', created_ts: 1, updated_ts: 2,
        turns: [{role: 'user', text: 'ok'}, {role: 'tool', text: 'nope'}, null],
    });
    assertEq(parseSession(raw).turns, [{role: 'user', text: 'ok', model: null}]);
    const noTurns = JSON.stringify(
        {id: 's-1', title: 't', created_ts: 1, updated_ts: 2});
    assertEq(parseSession(noTurns).turns, []);
});

// ---- the index -----------------------------------------------------

test('parseSessionIndex orders by activity, newest first', () => {
    const json = serializeSessionIndex([
        {id: 'a', title: 'a', created_ts: 1, updated_ts: 10},
        {id: 'c', title: 'c', created_ts: 3, updated_ts: 30},
        {id: 'b', title: 'b', created_ts: 2, updated_ts: 20},
    ]);
    assertEq(parseSessionIndex(json).map(e => e.id), ['c', 'b', 'a']);
});

test('parseSessionIndex keeps array order on same-millisecond ties', () => {
    const json = serializeSessionIndex([
        {id: 'first', title: 'x', created_ts: 5, updated_ts: 5},
        {id: 'second', title: 'y', created_ts: 5, updated_ts: 5},
    ]);
    assertEq(parseSessionIndex(json).map(e => e.id), ['first', 'second']);
});

test('a corrupt index degrades instead of breaking startup', () => {
    assertEq(parseSessionIndex(null), []);
    assertEq(parseSessionIndex(''), [], 'tombstoned namespace');
    assertEq(parseSessionIndex('not json'), []);
    assertEq(parseSessionIndex('{"sessions":1}'), []);
    assertEq(parseSessionIndex(JSON.stringify([
        {bogus: true},
        {id: '', title: 'x', created_ts: 1, updated_ts: 1},
        {id: 'ok', title: 'x', created_ts: 1, updated_ts: 1},
    ])).map(e => e.id), ['ok']);
});

test('upsertIndex moves a session to the front without duplicating it', () => {
    const index = [
        {id: 'b', title: 'b', created_ts: 2, updated_ts: 20},
        {id: 'a', title: 'a', created_ts: 1, updated_ts: 10},
    ];
    const bumped = upsertIndex(index,
        {id: 'a', title: 'a2', created_ts: 1, updated_ts: 30});
    assertEq(bumped.map(e => e.id), ['a', 'b']);
    assertEq(bumped[0].title, 'a2', 'the entry is replaced, not merged');
    assertEq(index.map(e => e.id), ['b', 'a'], 'the input is not mutated');

    const added = upsertIndex(index,
        {id: 'c', title: 'c', created_ts: 3, updated_ts: 40});
    assertEq(added.map(e => e.id), ['c', 'b', 'a']);
});

test('removeFromIndex drops exactly one session', () => {
    const index = [
        {id: 'b', title: 'b', created_ts: 2, updated_ts: 20},
        {id: 'a', title: 'a', created_ts: 1, updated_ts: 10},
    ];
    assertEq(removeFromIndex(index, 'b').map(e => e.id), ['a']);
    assertEq(removeFromIndex(index, 'nope').map(e => e.id), ['b', 'a']);
});

test('displayIndex pins an unwritten conversation to the front', () => {
    const index = [{id: 'a', title: 'a', created_ts: 1, updated_ts: 10}];
    const fresh = {id: 'new', title: UNTITLED, created_ts: 9, updated_ts: 9};
    assertEq(displayIndex(index, fresh).map(e => e.id), ['new', 'a']);
    // Opening a stored conversation must not reorder the list.
    assertEq(displayIndex(index, index[0]).map(e => e.id), ['a']);
    assertEq(displayIndex(index, null).map(e => e.id), ['a']);
});

// ---- titles and activity -------------------------------------------

test('titleFromTurns names a conversation after its first user turn', () => {
    assertEq(titleFromTurns([
        {role: 'assistant', text: 'ignored'},
        {role: 'user', text: '  what is\n an inode?  '},
        {role: 'user', text: 'and a dentry?'},
    ]), 'what is an inode?');
});

test('titleFromTurns clips long prompts on a word boundary', () => {
    const long = 'explain the difference between a page cache and a buffer cache';
    const title = titleFromTurns([{role: 'user', text: long}]);
    assert(title.length <= 43, `"${title}" stays short`);
    assert(title.endsWith('…'), 'clipped titles are marked');
    assert(!title.includes('  '), 'no dangling space before the ellipsis');
    assert(long.startsWith(title.slice(0, -1)), 'it is a prefix of the prompt');

    // No word boundary to cut on → hard clip rather than a one-word title.
    const wall = 'x'.repeat(80);
    assertEq(titleFromTurns([{role: 'user', text: wall}]),
        `${'x'.repeat(42)}…`);
});

test('titleFromTurns keeps the existing title without a user turn', () => {
    assertEq(titleFromTurns([], 'kept'), 'kept');
    assertEq(titleFromTurns([{role: 'assistant', text: 'hi'}]), UNTITLED);
    assertEq(titleFromTurns(null, ''), UNTITLED);
});

test('sessionWithTurns titles and bumps the record', () => {
    const info = {id: 's-1', title: UNTITLED, created_ts: 5, updated_ts: 5};
    const written = sessionWithTurns(info, [
        {role: 'user', text: 'hello there', widget: {}},
        {role: 'assistant', text: 'hi', model: 'qwen3'},
    ], 99);
    assertEq(written, {
        id: 's-1', title: 'hello there', created_ts: 5, updated_ts: 99,
        turns: [
            {role: 'user', text: 'hello there', model: null},
            {role: 'assistant', text: 'hi', model: 'qwen3'},
        ],
    });
    // The first user turn never moves, so rewrites keep the same title.
    assertEq(sessionWithTurns(written, written.turns, 120).title, 'hello there');
});

test('formatSessionTime reads as activity, not a timestamp', () => {
    const now = 1700000000000;
    const ago = ms => formatSessionTime(now - ms, now);
    assertEq(ago(5 * 1000), 'just now');
    assertEq(ago(5 * 60 * 1000), '5m ago');
    assertEq(ago(3 * 3600 * 1000), '3h ago');
    assertEq(ago(2 * 86400 * 1000), '2d ago');
    assertEq(ago(30 * 86400 * 1000), '2023-10-15');
    assertEq(formatSessionTime(0, now), '', 'no timestamp, no subtitle');
});

// ---- upgrade path ---------------------------------------------------

test('the pre-sessions conversation migrates into a first session', () => {
    const legacy = JSON.stringify([
        {role: 'user', text: 'how do I resize /var?', model: null},
        {role: 'assistant', text: 'grow it', model: 'qwen3'},
    ]);
    const session = migrateLegacyConversation(legacy, 500);
    assertEq(session.title, 'how do I resize /var?');
    assertEq(session.created_ts, 500);
    assertEq(session.turns, [
        {role: 'user', text: 'how do I resize /var?', model: null},
        {role: 'assistant', text: 'grow it', model: 'qwen3'},
    ]);
    assertEq(parseSession(serializeSession(session)).turns, session.turns,
        'the migrated session is a normal record');
});

test('nothing to migrate stays nothing', () => {
    assertEq(migrateLegacyConversation(null), null, 'contextd absent');
    assertEq(migrateLegacyConversation(''), null, 'never written');
    assertEq(migrateLegacyConversation('[]'), null, 'cleared conversation');
    assertEq(migrateLegacyConversation('not json'), null);
});

// ---- the flow the window drives -------------------------------------

test('sessions persist side by side and switch without bleeding', () => {
    // A hand-rolled Context1 namespace: get/set of strings, tombstone
    // on delete — exactly what the window has to work with.
    const kv = new Map();
    const get = k => kv.get(k) ?? '';
    const write = (session, index) => {
        const record = sessionWithTurns(session, session.turns, session.updated_ts);
        kv.set(sessionKey(record.id), serializeSession(record));
        const next = upsertIndex(index, record);
        kv.set(INDEX_KEY, serializeSessionIndex(next));
        return next;
    };

    let index = parseSessionIndex(get(INDEX_KEY));
    assertEq(index, [], 'a fresh namespace lists nothing');

    const a = {...newSession(UNTITLED, 100), turns: [
        {role: 'user', text: 'first chat'}]};
    index = write(a, index);
    const b = {...newSession(UNTITLED, 200), turns: [
        {role: 'user', text: 'second chat'}]};
    index = write(b, index);

    // Restart: the index survives and the most recent opens first.
    index = parseSessionIndex(get(INDEX_KEY));
    assertEq(index.map(e => e.title), ['second chat', 'first chat']);
    assertEq(parseSession(get(sessionKey(index[0].id))).turns,
        [{role: 'user', text: 'second chat', model: null}]);
    assertEq(parseSession(get(sessionKey(index[1].id))).turns,
        [{role: 'user', text: 'first chat', model: null}]);

    // Activity in the older one moves it to the top.
    index = write({...a, turns: [...a.turns,
        {role: 'assistant', text: 'back to you', model: 'qwen3'}],
    updated_ts: 300}, index);
    assertEq(index.map(e => e.title), ['first chat', 'second chat']);

    // Delete: tombstone the record, drop the index entry.
    kv.set(sessionKey(b.id), '');
    index = removeFromIndex(index, b.id);
    kv.set(INDEX_KEY, serializeSessionIndex(index));
    index = parseSessionIndex(get(INDEX_KEY));
    assertEq(index.map(e => e.id), [a.id]);
    assertEq(parseSession(get(sessionKey(b.id))), null, 'gone, not empty');
});

finish('assistant-sessions');
