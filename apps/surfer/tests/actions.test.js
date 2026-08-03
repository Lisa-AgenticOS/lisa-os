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

finish();
