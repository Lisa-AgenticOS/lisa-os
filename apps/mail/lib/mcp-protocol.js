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
        const fn = tools[name];
        if (!fn)
            return fail(-32601, `no tool ${JSON.stringify(name)}`);
        try {
            const out = await fn(req.params?.arguments ?? {});
            // The tag goes INSIDE the payload as well as on the
            // envelope: agentd's dispatcher unwraps content[0].text and
            // discards the envelope, which is how the browser's tag was
            // lost on its first on-device run (2026-07-29).
            //
            // The spread is BEFORE the tag, so a message cannot override
            // its own provenance with a crafted field — a body reading
            // `{"provenance":"user"}` is text, not a claim we honour.
            const tagged = {...out, provenance: PROVENANCE};
            return reply({
                content: [{type: 'text', text: JSON.stringify(tagged)}],
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
