// The JSON-RPC surface, without a socket (#146 Phase 3).
import {test, assertEq, finish} from '../../../shell/testing/harness.js';
import {handleRequest} from '../lib/mcp-protocol.js';

const TOOLS = {
    read_page: async () => ({title: 'T', text: 'from the web'}),
    boom: async () => { throw new Error('nope'); },
};
const call = (method, params, id = 1) => handleRequest({jsonrpc: '2.0', id, method, params}, TOOLS);

// The shared harness is synchronous, so an async test body would pass
// VACUOUSLY — fn() returns a promise, nothing throws, "ok". All awaits
// happen here at module top level; the test() closures then assert on
// settled values.
const rInit = await call('initialize', {});
const rNote = await handleRequest({jsonrpc: '2.0', method: 'notifications/initialized'}, TOOLS);
const rCall = await call('tools/call', {name: 'read_page', arguments: {}});
const rBoom = await call('tools/call', {name: 'boom', arguments: {}});
const rNoTool = await call('tools/call', {name: 'nope'});
const rNoMethod = await call('wat', {});
// #218: `tools[name]` walks the prototype chain, so every name on
// Object.prototype resolved to a function and got CALLED. `constructor`
// came back as a tagged success — a fail-open one layer under the code
// that argues for failing closed.
const rInherited = await Promise.all(
    ['constructor', 'toString', 'valueOf', 'hasOwnProperty', '__proto__',
     '__defineGetter__', 'isPrototypeOf', 'propertyIsEnumerable']
        .map(name => call('tools/call', {name})));
const rNonString = await Promise.all(
    [undefined, null, 42, {}, ['read_page']]
        .map(name => call('tools/call', {name})));
// A name that IS an own property but is not callable. Nothing in this
// app's tool map is a string today; the dispatcher should not be the
// thing that finds out the hard way.
const rNotCallable = await handleRequest(
    {jsonrpc: '2.0', id: 1, method: 'tools/call', params: {name: 'version'}},
    {version: '0.1', read_page: async () => ({})});
const rJunk = await handleRequest(null, TOOLS);
const rOldRpc = await handleRequest({jsonrpc: '1.0', id: 9}, TOOLS);

test('initialize answers with server info', () => {
    assertEq(rInit.result.serverInfo.name, 'app.lisaos.Surfer');
});

test('the initialized notification gets NO reply', () => {
    assertEq(rNote, null);
});

test('tools/call tags the ENVELOPE web, once (#313)', () => {
    assertEq(rCall.result.provenance, 'web');
    const payload = JSON.parse(rCall.result.content[0].text);
    assertEq(payload.title, 'T');
    // The payload used to carry a second copy, because the bus
    // dispatcher unwrapped content[0].text and dropped the envelope.
    // `mcp-bus`'s `carry_envelope` hoists envelope fields onto the
    // unwrapped payload now, so the duplicate is gone and the tag has
    // one home.
    assertEq(payload.provenance, undefined,
        'the payload copy of the tag is back — one tag, one place (#313)');
});

test('a throwing tool is an isError result, not a dead socket', () => {
    assertEq(rBoom.result.isError, true);
    assertEq(rBoom.result.content[0].text.includes('nope'), true);
});

test('an unknown tool and an unknown method are JSON-RPC errors', () => {
    assertEq(rNoTool.error.code, -32601);
    assertEq(rNoMethod.error.code, -32601);
});

test('inherited Object.prototype members are not tools (#218)', () => {
    for (const r of rInherited) {
        assertEq(r.error?.code, -32601,
            `an inherited member answered as a tool: ${JSON.stringify(r)}`);
    }
});

test('a tool name that is not a string is not a tool', () => {
    for (const r of rNonString)
        assertEq(r.error?.code, -32601, JSON.stringify(r));
});

test('an entry that is not callable is not a tool', () => {
    assertEq(rNotCallable.error?.code, -32601, JSON.stringify(rNotCallable));
});

test('junk input is an invalid-request error, not a crash', () => {
    assertEq(rJunk.error.code, -32600);
    assertEq(rOldRpc.error.code, -32600);
});

finish('surfer/mcp');
