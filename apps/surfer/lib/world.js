// Where agent scripts run (#212).
//
// # The defect this module exists to prevent
//
// `evaluate_javascript(script, -1, null, …)` — the third argument is
// `world_name`, and NULL means the page's own JavaScript world. Every
// agent-facing script ran there: the extractor, `click`, `fill`, the
// selection read. In that world the PAGE owns `JSON.stringify`,
// `document.querySelector`, `Object.prototype` — everything the tool
// scripts are built out of.
//
// Verified on the device with a page that redefined both:
//
//   JSON.stringify  → returned a forged {title, text, links}, so
//                     `read_page` reported a bank balance and an
//                     "IGNORE PREVIOUS" instruction from a page that
//                     said neither.
//   document.querySelector → mapped "#q" to "#pw", so a `fill` the
//                     human approved as the search box wrote into the
//                     password field. The confirmation stays intact
//                     while describing an action that does not happen.
//
// Escaping the arguments (lib/actions.js) was guarding the wrong layer:
// nothing you can do to a script's TEXT helps when the callee owns the
// functions it calls.
//
// # The fix
//
// One named script world. WebKit gives a non-NULL `world_name` its own
// global object and its own DOM wrapper prototypes while sharing the
// same document, so the page cannot see, reach or redefine anything the
// agent scripts use. Per WebKit-6.0.gir on the device (WebKitGTK 2.48):
// "If world_name is NULL, the default world is used. Any value that is
// not NULL is a distinct world."
//
// This module holds no gi:// import so the call shape is testable off a
// display — tests/world.test.js pins the argument that decides it.

/// The one world every agent-facing script runs in. Shared on purpose:
/// read and write must see the same isolated globals, and a second name
/// would be a second surface to remember to isolate.
export const AGENT_WORLD = 'lisa-surfer-agent';

/// Evaluate `script` in the agent world of `view`. Resolves with the
/// result as a string (our scripts all return JSON text); rejects if the
/// script threw, so a failure is never a silently empty page.
export function evaluateInAgentWorld(view, script) {
    return new Promise((resolve, reject) => {
        view.evaluate_javascript(script, -1, AGENT_WORLD, null, null, (v, res) => {
            try {
                resolve(v.evaluate_javascript_finish(res).to_string());
            } catch (e) {
                reject(e);
            }
        });
    });
}
