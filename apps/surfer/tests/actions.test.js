// Write-tier action rules (#166): what navigate may open, and how
// click/fill embed hostile arguments into page script.
import {test, assert, assertEq, finish} from '../../../shell/testing/harness.js';
import {navigationTarget, clickScript, fillScript} from '../lib/actions.js';

test('https passes through untouched', () => {
    assertEq(navigationTarget('https://example.org/a?b=c'), 'https://example.org/a?b=c');
});

test('javascript: is refused, case and whitespace included', () => {
    for (const bad of ['javascript:alert(1)', ' JaVaScRiPt:alert(1)', 'data:text/html,<script>1</script>', 'blob:x', 'vbscript:x']) {
        let threw = false;
        try { navigationTarget(bad); } catch { threw = true; }
        assert(threw, `must refuse ${bad}`);
    }
});

test('the agent boundary allows http and https, and nothing else (#214)', () => {
    // The address bar's passthrough list is the ADDRESS BAR's rule: a
    // person browsing their own machine to file:///home/… is their
    // business (ADR-0029's second test). The agent boundary is a
    // different question with a different answer — `navigate
    // file:///etc/passwd` then `read_page` is any readable file, tagged
    // provenance "web", straight into the model's context.
    assertEq(navigationTarget('http://example.org/'), 'http://example.org/');
    assertEq(navigationTarget('example.org'), 'https://example.org');
    for (const bad of [
        'file:///etc/passwd',
        'file:///home/lisa/.ssh/id_ed25519',
        'FILE:///etc/shadow',
        'about:blank',
        'about:config',
        'mailto:a@b.com',
        'magnet:?xt=urn:btih:abc',
        'ftp://example.org/x',
        'lisa://start',
    ]) {
        let threw = false;
        try { navigationTarget(bad); } catch { threw = true; }
        assert(threw, `agent must not be able to open ${bad}`);
    }
});

test('the refusal says what IS allowed', () => {
    let msg = '';
    try { navigationTarget('file:///etc/passwd'); } catch (e) { msg = e.message; }
    assert(msg.includes('http'), `unhelpful refusal: ${msg}`);
});

test('a search-looking input is refused rather than searched', () => {
    // A person typing words gets a search; an agent must say where it
    // wants to GO. "weather today" navigating to a search engine would
    // be a write the consent dialog never described.
    let threw = false;
    try { navigationTarget('weather today'); } catch { threw = true; }
    assert(threw, 'free text must not become a navigation');
});

test('selectors and values are data, not script', () => {
    // The classic breakouts: a quote to close the literal, a </script>
    // to close a tag context, a backslash to eat the closing quote.
    for (const hostile of ['"];alert(1);//', '</script><script>1', 'a\\', "input[name='q']"]) {
        const s = clickScript(hostile) + fillScript(hostile, hostile);
        // The hostile text must only ever appear JSON-escaped: no raw
        // `</script>` and no unescaped double-quote-bracket breakout.
        assert(!s.includes('</script>'), `raw </script> leaked for ${hostile}`);
    }
    // And the scripts stay syntactically balanced: a naive concat would
    // unbalance the parens/braces counts on the quote-breakout case.
    const probe = fillScript('"];x;//', 'v');
    assertEq((probe.match(/\(/g) ?? []).length, (probe.match(/\)/g) ?? []).length);
});

test('fill dispatches input and change so framework pages notice', () => {
    const s = fillScript('#q', 'hello');
    assert(s.includes("new Event('input'"), 'input event missing');
    assert(s.includes("new Event('change'"), 'change event missing');
    assert(s.includes('isContentEditable'), 'contenteditable branch missing');
});

test('click reports a miss instead of staying silent', () => {
    assert(clickScript('#nope').includes('no element matches'));
});

finish('surfer/actions');
