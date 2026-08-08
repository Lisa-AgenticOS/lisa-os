// Unit tests for the Assistant's mode model (the navrail's modes:
// rail | sidebar | chat screen). Pure data + fallbacks; every mode must
// carry the real effects a mode is (rule 10 — never a dead button).
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    MODE_IDS, MODES, DEFAULT_MODE, modeById, wireMode, needsWorkspace,
} from '../lib/modes.js';

test('every listed mode id has a full definition', () => {
    for (const id of MODE_IDS) {
        const m = MODES[id];
        assert(m, `no definition for mode ${id}`);
        assertEq(m.id, id);
        for (const field of ['label', 'icon', 'placeholder', 'summary'])
            assert(typeof m[field] === 'string' && m[field].length > 0,
                `mode ${id} missing ${field}`);
        assert(typeof m.needsWorkspace === 'boolean',
            `mode ${id} needsWorkspace not boolean`);
    }
});

test('the four modes the owner named are present and ordered', () => {
    assertEq(MODE_IDS, ['chat', 'code', 'design', 'research']);
});

test('modeById falls back to the default rather than throwing', () => {
    assertEq(modeById('code').id, 'code');
    assertEq(modeById('no-such-mode').id, DEFAULT_MODE);
    assertEq(modeById(undefined).id, DEFAULT_MODE);
    assertEq(DEFAULT_MODE, 'chat', 'the fallback must be the safe general surface');
});

test('only Code requires a working folder', () => {
    assert(needsWorkspace('code'), 'Code must require a workspace');
    for (const id of ['chat', 'design', 'research'])
        assert(!needsWorkspace(id), `${id} must not require a workspace`);
    // An unknown mode does not accidentally demand one.
    assert(!needsWorkspace('no-such-mode'));
});

test('wireMode is a plain validated id, unknown collapses to the default', () => {
    assertEq(wireMode('research'), 'research');
    assertEq(wireMode('../../etc'), DEFAULT_MODE);
    // No metacharacters ever reach the wire — it is one of the known ids.
    for (const id of ['chat', 'code', 'design', 'research'])
        assert(MODE_IDS.includes(wireMode(id)));
});

finish('assistant modes');
