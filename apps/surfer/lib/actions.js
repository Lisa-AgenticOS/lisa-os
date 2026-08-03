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

/// Where `navigate` may actually go.
///
/// Delegates to resolveInput — the ONE place that refuses javascript:,
/// data:, vbscript: and blob: before normalising (its refusal list was
/// written for exactly this tool). A search-looking input is refused
/// rather than searched: a person typing gets a search, an agent
/// navigating gets told to say where it wants to go.
export function navigationTarget(raw) {
    const resolved = resolveInput(raw);
    if (resolved.kind === 'refused')
        throw new Error(resolved.reason);
    if (resolved.kind !== 'load' || !resolved.url)
        throw new Error(`not a navigable address: ${JSON.stringify(String(raw ?? ''))}`);
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
export function fillScript(selector, value) {
    return `(() => {
        try {
            const el = document.querySelector(${asLiteral(selector)});
            if (!el)
                return JSON.stringify({filled: false, reason: 'no element matches'});
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
