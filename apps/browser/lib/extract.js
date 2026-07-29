// Page extraction (ADR-0037 §2, issue #146 Phase 2).
//
// EXTRACT_JS runs inside the page via evaluate_javascript; pageResult()
// shapes what came back, pure and testable. The ceiling exists because
// an unbounded page dump costs the model the context it needed to act
// on the page — the same lesson as the shell tool's output cap.

/// Runs in the page. Collects what an agent needs and nothing exotic:
/// title, readable text, and the links (an agent that cannot see hrefs
/// cannot navigate anywhere real).
export const EXTRACT_JS = `(() => {
    const links = [...document.querySelectorAll('a[href]')].slice(0, 200)
        .map(a => ({text: (a.innerText || '').trim().slice(0, 120), href: a.href}))
        .filter(l => l.text);
    return JSON.stringify({
        title: document.title,
        text: document.body ? document.body.innerText : '',
        links,
    });
})()`;

/// Character budget for the text body. Beyond it the result says so
/// rather than silently ending — a truncation the model cannot see is a
/// page it thinks it has read.
export const MAX_TEXT_CHARS = 30000;

/// Raw page JSON + the engine's own URI → the tool result.
export function pageResult(raw, url) {
    const text = typeof raw.text === 'string' ? raw.text : '';
    const truncated = text.length > MAX_TEXT_CHARS;
    return {
        url: url ?? null,
        title: typeof raw.title === 'string' ? raw.title : '',
        text: truncated ? text.slice(0, MAX_TEXT_CHARS) : text,
        truncated,
        links: Array.isArray(raw.links) ? raw.links.slice(0, 200) : [],
    };
}
