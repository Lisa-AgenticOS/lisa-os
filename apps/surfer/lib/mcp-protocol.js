// The JSON-RPC protocol surface, pure (#146 Phase 3). Split from the
// socket so it can be tested under node, which cannot load gi:// —
// and so the provenance tag lives in a module with no I/O in it.

export const APP_ID = 'app.lisaos.Surfer';

/// Pure: one decoded JSON-RPC request → the reply object (or null for a
/// notification). `tools` maps name → async handler. Split from the
/// socket so the protocol is testable without one.
export async function handleRequest(req, tools) {
    if (!req || req.jsonrpc !== '2.0')
        return {jsonrpc: '2.0', id: req?.id ?? null, error: {code: -32600, message: 'invalid request'}};
    const reply = (result) => ({jsonrpc: '2.0', id: req.id, result});
    const fail = (code, message) => ({jsonrpc: '2.0', id: req.id, error: {code, message}});

    switch (req.method) {
    case 'initialize':
        return reply({
            protocolVersion: '2024-11-05',
            serverInfo: {name: APP_ID, version: '0.1'},
            capabilities: {tools: {}},
        });
    case 'notifications/initialized':
        return null; // notification: no reply at all
    case 'tools/call': {
        const name = req.params?.name;
        // Own properties only (#218). `tools[name]` walked the prototype
        // chain, so `constructor`, `toString` and every other member of
        // Object.prototype resolved to a real function and got CALLED —
        // and answered with a tagged SUCCESS where the protocol says
        // -32601. A dispatcher that fails open is a strange floor to
        // build a guard on.
        const fn = typeof name === 'string' &&
            Object.prototype.hasOwnProperty.call(tools, name)
            ? tools[name] : undefined;
        if (typeof fn !== 'function')
            return fail(-32601, `no tool ${JSON.stringify(name)}`);
        try {
            const out = await fn(req.params?.arguments ?? {});
            // The tag goes on the ENVELOPE, once (#313). It used to go
            // on the envelope AND inside the payload, because the bus
            // dispatcher unwrapped content[0].text and threw the
            // envelope away — which is how the first on-device run lost
            // the tag (2026-07-29). `mcp-bus`'s `carry_envelope` now
            // hoists envelope fields onto the unwrapped payload, and it
            // lets the envelope win a collision, so a page that echoes
            // `{"provenance":"user"}` back through a handler still
            // arrives as web content. One tag, one place, and the fourth
            // app to be written gets the behaviour without reading this
            // comment.
            return reply({content: [{type: 'text', text: JSON.stringify(out)}], provenance: 'web'});
        } catch (e) {
            return reply({content: [{type: 'text', text: `error: ${e.message ?? e}`}], isError: true});
        }
    }
    default:
        return req.id === undefined ? null : fail(-32601, `unknown method ${req.method}`);
    }
}

