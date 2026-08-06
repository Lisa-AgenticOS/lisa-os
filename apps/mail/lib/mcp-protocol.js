// The JSON-RPC protocol surface, pure — same shape as Surfer's, with
// one thing changed and it is the important one: the provenance tag.
//
// Mail is the most consequential context source Lisa has. A web page is
// something you chose to open; a message is something anyone can send
// you, and its entire text is attacker-controlled by construction. That
// is exactly what a `mail` provenance tag is for: agentd escalates the
// confirmation tier of any privileged call whose chain includes it
// (PLAN §5.10, Appendix C), so "summarise my mail and then delete
// something" asks before it deletes.
//
// Split from the socket so it is testable under node, which cannot load
// gi://.

export const APP_ID = 'app.lisaos.Mail';

/// Everything this app emits is mail-provenance. Not a parameter, not
/// read from the message, not overridable by a handler: a constant,
/// applied here, on the way out.
const PROVENANCE = 'mail';

/// Pure: one decoded JSON-RPC request → the reply object (or null for a
/// notification). `tools` maps name → async handler.
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
        return null;
    case 'tools/call': {
        const name = req.params?.name;
        // Own properties only (#218). `tools[name]` walked the prototype
        // chain, so `constructor`, `toString` and every other member of
        // Object.prototype resolved to a real function and got CALLED,
        // answering with a tagged SUCCESS where the protocol says
        // -32601. The same line was wrong in Surfer and Preview.
        const fn = typeof name === 'string' &&
            Object.prototype.hasOwnProperty.call(tools, name)
            ? tools[name] : undefined;
        if (typeof fn !== 'function')
            return fail(-32601, `no tool ${JSON.stringify(name)}`);
        try {
            const out = await fn(req.params?.arguments ?? {});
            // The tag goes on the ENVELOPE, once (#313). It used to go
            // inside the payload as well, because the bus dispatcher
            // unwrapped content[0].text and discarded the envelope —
            // how the browser's tag was lost on its first on-device run
            // (2026-07-29). `mcp-bus`'s `carry_envelope` hoists envelope
            // fields onto the unwrapped payload now, and the envelope
            // wins a collision, so a body reading `{"provenance":"user"}`
            // is still text rather than a claim we honour.
            return reply({
                content: [{type: 'text', text: JSON.stringify(out)}],
                provenance: PROVENANCE,
            });
        } catch (e) {
            return reply({content: [{type: 'text', text: `error: ${e.message ?? e}`}], isError: true});
        }
    }
    default:
        return req.id === undefined ? null : fail(-32601, `unknown method ${req.method}`);
    }
}
