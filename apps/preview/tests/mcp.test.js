// Preview's JSON-RPC dispatch (#218).
//
// The same `tools[name]` prototype walk Surfer and Mail had: every
// member of Object.prototype resolved to a function and was called, so
// `constructor` came back as a tagged SUCCESS where the protocol says
// -32601. Fail-open one layer under the code that argues for failing
// closed.
import {test, assertEq, finish} from '../../../shell/testing/harness.js';
import {APP_ID, handleRequest} from '../lib/mcp-protocol.js';

const call = (name, args = {}) =>
    ({jsonrpc: '2.0', id: 1, method: 'tools/call', params: {name, arguments: args}});

test('inherited Object.prototype members are not tools', async () => {
    for (const name of [
        'constructor', 'toString', 'valueOf', 'hasOwnProperty',
        '__proto__', '__defineGetter__', 'isPrototypeOf',
    ]) {
        const out = await handleRequest(call(name), {open_document: async () => ({})});
        assertEq(out.error?.code, -32601,
            `${name} answered as a tool: ${JSON.stringify(out)}`);
    }
});

test('a tool name that is not a string is not a tool', async () => {
    for (const name of [undefined, null, 42, {}, ['open_document']]) {
        const out = await handleRequest(call(name), {open_document: async () => ({})});
        assertEq(out.error?.code, -32601, JSON.stringify(out));
    }
});

test('a real tool still answers, tagged file provenance', async () => {
    // The positive control: the fix must refuse inherited names without
    // refusing the tools that exist.
    const out = await handleRequest(call('open_document'), {
        open_document: async () => ({pages: 3}),
    });
    assertEq(out.result.provenance, 'file');
    assertEq(JSON.parse(out.result.content[0].text).pages, 3);
});

test('initialize names this app', async () => {
    const init = await handleRequest({jsonrpc: '2.0', id: 0, method: 'initialize'}, {});
    assertEq(init.result.serverInfo.name, APP_ID);
});

await finish('preview/mcp');
