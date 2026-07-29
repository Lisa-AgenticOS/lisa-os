// The JSON-RPC protocol surface, pure (#146 Phase 3). Split from the
// socket so it can be tested under node, which cannot load gi:// —
// and so the provenance tag lives in a module with no I/O in it.

export const APP_ID = 'app.lisaos.Browser';

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
        const fn = tools[name];
        if (!fn)
            return fail(-32601, `no tool ${JSON.stringify(name)}`);
        try {
            const out = await fn(req.params?.arguments ?? {});
            // The tag goes INSIDE the payload, not (only) on the JSON-RPC
            // envelope: agentd's dispatcher unwraps content[0].text and
            // discards the envelope, which is exactly how the first
            // on-device run lost the tag (2026-07-29). Content assembled
            // from a web page is web-provenance by definition, whatever
            // the page said about itself — and the spread is AFTER `out`,
            // so a page cannot override the tag via a crafted field.
            const tagged = {...out, provenance: 'web'};
            return reply({content: [{type: 'text', text: JSON.stringify(tagged)}], provenance: 'web'});
        } catch (e) {
            return reply({content: [{type: 'text', text: `error: ${e.message ?? e}`}], isError: true});
        }
    }
    default:
        return req.id === undefined ? null : fail(-32601, `unknown method ${req.method}`);
    }
}

