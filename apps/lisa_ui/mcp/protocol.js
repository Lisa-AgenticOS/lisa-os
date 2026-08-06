// The JSON-RPC/MCP protocol surface for a Lisa app — ONE copy
// (ADR-0056 step 1, ADR-0047 §6).
//
// # Why this file exists
//
// Mail, Surfer and Preview each carried their own `mcp-protocol.js`.
// They were copies once; by 2026-08-06 they had DRIFTED — three hashes,
// 72/67/60 lines — and the drift was not cosmetic. Measured, not
// assumed, when they were finally diffed with comments stripped:
//
//   * Preview answered a thrown tool with `fail(-32000, …)`, a
//     JSON-RPC *protocol* error. Mail and Surfer answered with a
//     RESULT carrying `isError: true`. MCP reserves protocol errors for
//     protocol problems — a tool that throws is a tool result.
//   * Preview replied to NOTIFICATIONS. JSON-RPC 2.0 §4.1: a server
//     MUST NOT reply to a request with no `id`. Mail and Surfer
//     returned null.
//   * All three dropped the provenance tag on the error path, so
//     `error: ${e.message}` — which can carry a filename or subject
//     line an attacker chose — reached the model with no tag at all.
//     The same hole as #313, through the error door.
//
// PLAN §5.12 already recorded the cost of the triplication: it is why
// #218 had to be found and fixed three times. This is the fourth
// instance of the pattern, and the reason ADR-0056 makes this edge step
// one rather than a widget set.
//
// # Why it lives under apps/ and not libs/
//
// ADR-0056 names `libs/lisa_ui`. The staged tree decides otherwise:
// `build-apps-payload.sh` flattens `apps/<app>` and `shell/<surface>`
// into ONE root, so `mail/lib/x.js` is two levels from the root when
// shipped and three when in the repo. A relative import can only mean
// the same thing in both trees if the library sits beside the apps, so
// it does. Every consumer of THIS module is an app; the day a shell
// surface needs `lisa_ui`, that depth question has to be answered
// properly rather than papered over, and this comment is the warning.
//
// # What is app-specific, and what is not
//
// Two things, both constants, both passed in: the app id and the
// provenance tag. Everything else is protocol and is identical by
// construction now. The tag is applied HERE, on the way out — never
// read from the request, never returned by a handler, never a
// parameter a tool can influence.

/// Build the pure request handler for one app.
///
/// `appId`     — reverse-DNS id, reported in `initialize`.
/// `provenance` — the tag every result from this app carries.
///
/// Returns `handleRequest(req, tools)`: one decoded JSON-RPC request →
/// the reply object, or `null` for a notification. `tools` maps name →
/// async handler.
export function makeHandler({appId, provenance}) {
    if (typeof appId !== 'string' || !appId)
        throw new Error('lisa_ui/mcp: appId is required');
    // A missing tag must not silently mean "untagged". An app whose
    // output reaches the model with no provenance is an app whose
    // output is treated as trusted, which is the failure this whole
    // edge exists to prevent — so it is a construction-time error, not
    // a runtime surprise on somebody's desktop.
    if (typeof provenance !== 'string' || !provenance)
        throw new Error(`lisa_ui/mcp: ${appId} must declare a provenance tag`);

    return async function handleRequest(req, tools) {
        if (!req || req.jsonrpc !== '2.0') {
            return {
                jsonrpc: '2.0',
                id: req?.id ?? null,
                error: {code: -32600, message: 'invalid request'},
            };
        }
        const reply = (result) => ({jsonrpc: '2.0', id: req.id, result});
        const fail = (code, message) => ({jsonrpc: '2.0', id: req.id, error: {code, message}});
        // JSON-RPC 2.0 §4.1: no id means a notification, and a server
        // MUST NOT reply to one. Preview did, for every unknown method.
        const isNotification = req.id === undefined;

        switch (req.method) {
        case 'initialize':
            return reply({
                protocolVersion: '2024-11-05',
                serverInfo: {name: appId, version: '0.1'},
                capabilities: {tools: {}},
            });
        case 'notifications/initialized':
            return null;
        case 'tools/call': {
            const name = req.params?.name;
            // Own properties only (#218). `tools[name]` walked the
            // prototype chain, so `constructor`, `toString` and every
            // other member of Object.prototype resolved to a real
            // function and got CALLED, answering with a tagged SUCCESS
            // where the protocol says -32601. A dispatcher that fails
            // open is a strange floor to build a guard on. This bug was
            // fixed three times, in three files; now there is one.
            const fn = typeof name === 'string' &&
                Object.prototype.hasOwnProperty.call(tools, name)
                ? tools[name] : undefined;
            if (typeof fn !== 'function')
                return isNotification ? null : fail(-32601, `no tool ${JSON.stringify(name)}`);
            try {
                const out = await fn(req.params?.arguments ?? {});
                // The tag goes on the ENVELOPE, once (#313). It used to
                // go inside the payload as well, because the bus
                // dispatcher unwrapped content[0].text and discarded
                // the envelope — how Surfer's tag was lost on its first
                // on-device run. `mcp-bus`'s `carry_envelope` hoists
                // envelope fields onto the unwrapped payload now, and
                // the envelope WINS a collision, so a document that
                // contains `{"provenance":"user"}` is text, not a claim
                // anybody honours.
                return reply({
                    content: [{type: 'text', text: JSON.stringify(out)}],
                    provenance,
                });
            } catch (e) {
                // A tool that throws is a TOOL RESULT with isError, not
                // a protocol error: the call reached the tool, and the
                // model is entitled to see why it failed. Preview
                // returned -32000 here, which reads to a client as "no
                // such method" rather than "your argument was wrong".
                //
                // And it is tagged. The message can quote a filename, a
                // subject line or a page title that an attacker chose,
                // so it is exactly as untrusted as a success — the
                // error path used to be the one door out of this edge
                // with no tag on it.
                return reply({
                    content: [{type: 'text', text: `error: ${e?.message ?? String(e)}`}],
                    isError: true,
                    provenance,
                });
            }
        }
        default:
            return isNotification ? null : fail(-32601, `unknown method ${req.method}`);
        }
    };
}
