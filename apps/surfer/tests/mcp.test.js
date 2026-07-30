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
const rJunk = await handleRequest(null, TOOLS);
const rOldRpc = await handleRequest({jsonrpc: '1.0', id: 9}, TOOLS);

test('initialize answers with server info', () => {
    assertEq(rInit.result.serverInfo.name, 'app.lisaos.Surfer');
});

test('the initialized notification gets NO reply', () => {
    assertEq(rNote, null);
});

test('tools/call tags the PAYLOAD web — the envelope gets stripped downstream', () => {
    const payload = JSON.parse(rCall.result.content[0].text);
    assertEq(payload.provenance, 'web',
        'agentd unwraps content[0].text and drops the envelope; the tag must survive that');
    assertEq(payload.title, 'T');
    assertEq(rCall.result.provenance, 'web');
});

test('a throwing tool is an isError result, not a dead socket', () => {
    assertEq(rBoom.result.isError, true);
    assertEq(rBoom.result.content[0].text.includes('nope'), true);
});

test('an unknown tool and an unknown method are JSON-RPC errors', () => {
    assertEq(rNoTool.error.code, -32601);
    assertEq(rNoMethod.error.code, -32601);
});

test('junk input is an invalid-request error, not a crash', () => {
    assertEq(rJunk.error.code, -32600);
    assertEq(rOldRpc.error.code, -32600);
});

finish('surfer/mcp');
