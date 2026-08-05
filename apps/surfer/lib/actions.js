// Write-tier agent actions (#166, ADR-0037): the pure halves of
// navigate / click / fill. No GNOME imports, so every rule here runs
// under any JS engine in tests — the app wires these to WebKit.
//
// # The security shape, stated once
//
// These are Write-tier tools driven by a model that has usually just
// READ the page it is now acting on. Page content is `web` provenance
// — untrusted by construction — so agentd escalates the call to the
// consent surface before it reaches us (ADR-0029/0030; the guard is
// deterministic code in agentd, never prompt text here). What THIS
// module owns is narrower: whatever arrives must not be able to smuggle
// script where an argument belongs.

import {resolveInput} from './url.js';
import {credentialGuardPreamble} from './credentials.js';

/// The only schemes an AGENT may open. An allowlist, not a blocklist.
///
/// This is the fix for #214, and the shape of that bug is worth keeping
/// written down: `navigate` used to inherit the address bar's
/// passthrough list, which includes `file:` and `about:` because a
/// PERSON may open their own files. Reused at the agent boundary it
/// meant `navigate file:///etc/passwd` followed by `read_page` returned
/// the contents of any file the user can read, tagged
/// `provenance: "web"`, straight into the model's context and around
/// contextd's ACLs entirely.
///
/// Two rules, two answers (ADR-0029): the address bar sits between a
/// person and their own machine and stays open; this sits between the
/// model and the machine and is closed by default. `about:` is not here
/// either — nothing an agent needs to do requires it, and "harmless
/// today" is how the file: hole got in.
const AGENT_SCHEMES = ['http:', 'https:'];

/// Where `navigate` may actually go.
///
/// Two gates, in order. resolveInput first — the ONE place that refuses
/// javascript:, data:, vbscript: and blob: before normalising, and the
/// place that turns `example.org` into `https://example.org`. Then the
/// agent allowlist above, applied to what came out, because the refusal
/// list alone is a blocklist and a blocklist at a trust boundary is a
/// list of the attacks somebody thought of.
///
/// A search-looking input is refused rather than searched: a person
/// typing gets a search, an agent navigating gets told to say where it
/// wants to go.
export function navigationTarget(raw) {
    const resolved = resolveInput(raw);
    if (resolved.kind === 'refused')
        throw new Error(resolved.reason);
    if (resolved.kind !== 'load' || !resolved.url)
        throw new Error(`not a navigable address: ${JSON.stringify(String(raw ?? ''))}`);
    const lower = resolved.url.toLowerCase();
    if (!AGENT_SCHEMES.some(s => lower.startsWith(s))) {
        throw new Error(
            `an agent may only open ${AGENT_SCHEMES.join(' and ')} addresses, ` +
            `not ${JSON.stringify(resolved.url)}`);
    }
    return resolved.url;
}

/// A CSS selector, embedded into page JS as data.
///
/// JSON.stringify is the whole escaping story: the selector becomes a
/// JS string literal whose quotes, backslashes and even `</script>`
/// are inert. querySelector then treats it as a selector or throws its
/// own SyntaxError — which the script reports instead of executing
/// anything the selector said.
function asLiteral(value) {
    // JSON.stringify handles quotes and backslashes but passes `<`
    // through — fine for evaluate_javascript today, but the day this
    // string reaches an HTML context, a literal </script> ends the
    // script element mid-literal. < is the same character with no
    // HTML meaning anywhere.
    return JSON.stringify(String(value ?? '')).replace(/</g, '\\u003c');
}

/// Page script for `click(selector)`. Reports what it did — a click on
/// nothing must come back "no match", not silence.
export function clickScript(selector) {
    return `(() => {
        try {
            const el = document.querySelector(${asLiteral(selector)});
            if (!el)
                return JSON.stringify({clicked: false, reason: 'no element matches'});
            el.click();
            return JSON.stringify({clicked: true,
                element: (el.tagName || '').toLowerCase(),
                text: (el.innerText || el.value || '').slice(0, 120)});
        } catch (e) {
            return JSON.stringify({clicked: false, reason: String(e.message || e)});
        }
    })()`;
}

/// Page script for `fill(selector, value)`.
///
/// Sets the property AND dispatches input+change: framework pages
/// (React et al.) read state from the events, not the attribute, and a
/// fill that skips them "works" on plain HTML while silently doing
/// nothing on half the real web.
///
/// **The credential check is INSIDE this script, not in front of it**
/// (#260). Classifying the element in one `evaluate_javascript` call
/// and filling it in a second leaves a gap in which the page can swap
/// the element — the ADR-0033 shape, and the reason `lib/target.js`
/// exists. Here the resolve, the classification and the write happen in
/// one synchronous turn with no `await` between them, so no page script
/// runs in the middle. The detector's source comes from
/// `lib/credentials.js` (one copy, `tests/credentials.test.js` proves
/// it is the same one), and there is deliberately no unguarded fill
/// script for anything to call by mistake.
export function fillScript(selector, value) {
    return `(() => {
        ${credentialGuardPreamble()}
        try {
            const el = document.querySelector(${asLiteral(selector)});
            if (!el)
                return JSON.stringify({filled: false, reason: 'no element matches'});
            const why = isCredentialField(describeField(el));
            if (why !== null)
                return JSON.stringify(credentialRefusal(why));
            const v = ${asLiteral(value)};
            if (el.isContentEditable) {
                el.textContent = v;
            } else if ('value' in el) {
                el.value = v;
            } else {
                return JSON.stringify({filled: false,
                    reason: 'element is not a form field'});
            }
            el.dispatchEvent(new Event('input', {bubbles: true}));
            el.dispatchEvent(new Event('change', {bubbles: true}));
            return JSON.stringify({filled: true,
                element: (el.tagName || '').toLowerCase(),
                name: el.name || el.id || ''});
        } catch (e) {
            return JSON.stringify({filled: false, reason: String(e.message || e)});
        }
    })()`;
}
