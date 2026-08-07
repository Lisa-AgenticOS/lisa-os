// narrowIface: the subset can hold nothing the sdk copy does not serve,
// and asking for a vanished member fails loudly at import time.
import test from 'node:test';
import assert from 'node:assert/strict';
import {narrowIface} from '../bus/narrow.js';
import {IFACE_XML} from '../bus/interfaces.js';

test('the memory pane subset carries exactly what it asked for', () => {
    const xml = narrowIface('Harness1',
        ['MemoryList', 'MemoryForget', 'MemoryForgetAll']);
    assert.match(xml, /<interface name="dev\.lisaos\.Harness1">/);
    for (const m of ['MemoryList', 'MemoryForget', 'MemoryForgetAll'])
        assert.match(xml, new RegExp(`<method name="${m}">`));
    // The narrow view must NOT smuggle the rest of the interface in: a
    // surface holding this proxy cannot start or cancel runs.
    for (const absent of ['Run', 'Cancel', 'Token', 'Finished'])
        assert.doesNotMatch(xml, new RegExp(`name="${absent}"`));
    // Args survive the cut — a method without its args is a different
    // method as far as the wire is concerned.
    assert.match(xml, /<arg name="note_id" type="x" direction="in"\/>/);
});

test('a member the daemon stopped serving fails at import, not on a device', () => {
    assert.throws(() => narrowIface('Harness1', ['MemoryList', 'Vanished']),
        /has no member Vanished/);
});

test('an unknown interface and an empty member list are refused', () => {
    assert.throws(() => narrowIface('NoSuch1', ['Ping']), /unknown interface/);
    assert.throws(() => narrowIface('Harness1', []), /non-empty member list/);
});

test('signals narrow too, since a listener is also a capability', () => {
    const xml = narrowIface('Overlay1', ['GetStatus', 'Started']);
    assert.match(xml, /<signal name="Started">/);
    assert.doesNotMatch(xml, /name="Ask"/);
});

test('the sdk copy this narrows from is the introspected one', () => {
    // Guard the guard: if Memory* ever leaves the snapshot, the pane
    // test above would fail too — but this failure names the real
    // culprit (the snapshot) rather than the helper.
    assert.match(IFACE_XML.Harness1, /MemoryList/);
});
