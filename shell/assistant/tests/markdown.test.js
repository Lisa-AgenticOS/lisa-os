// Pango markup is XML, and invalid markup renders as NOTHING — not an
// error, not plain text, an empty label. So most of these are about
// input that must never become broken markup.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {escapeMarkup, toPangoMarkup} from '../lib/markdown.js';

test('the characters that break Pango are escaped, once', () => {
    // The failure this prevents: a model explaining HTML, and the whole
    // reply vanishing.
    const out = toPangoMarkup('use <script> and a & b');
    assert(out.includes('&lt;script&gt;'), out);
    assert(out.includes('a &amp; b'), out);
    // Once, not twice: &amp;amp; is what double-escaping looks like on
    // screen and it is a common way to get this wrong.
    assert(!out.includes('&amp;amp;'), out);
    assert(!out.includes('&amp;lt;'), out);
});

test('emphasis becomes emphasis', () => {
    assertEq(toPangoMarkup('**bold**'), '<b>bold</b>');
    assertEq(toPangoMarkup('__bold__'), '<b>bold</b>');
    assertEq(toPangoMarkup('*it*'), '<i>it</i>');
    assertEq(toPangoMarkup('_it_'), '<i>it</i>');
    // Bold is matched before italic; otherwise ** produces nested,
    // empty italics.
    assert(!toPangoMarkup('**bold**').includes('<i>'));
});

test('an underscore inside a word is not italics', () => {
    // Models write snake_case constantly. Italicising half an
    // identifier is worse than not italicising anything.
    assertEq(toPangoMarkup('call read_file_now here'), 'call read_file_now here');
    assertEq(toPangoMarkup('a_b_c'), 'a_b_c');
});

test('code is monospace and never parsed as markdown', () => {
    // A reply about Markdown is a normal reply, and its examples must
    // survive intact.
    const out = toPangoMarkup('run `ls **not bold**` now');
    assert(out.includes('<tt>ls **not bold**</tt>'), out);
    // …and code containing markup characters is escaped inside <tt>.
    assert(toPangoMarkup('`a < b`').includes('<tt>a &lt; b</tt>'));
});

test('a fenced block survives with its newlines and its markup characters', () => {
    const out = toPangoMarkup('before\n```sh\nif [ a < b ]; then\n  echo **hi**\nfi\n```\nafter');
    assert(out.includes('if [ a &lt; b ]; then'), out);
    assert(out.includes('echo **hi**'), out);
    assert(out.includes('<tt>'), out);
    assert(out.startsWith('before'), out);
    assert(out.trimEnd().endsWith('after'), out);
});

test('headings and lists get a shape Pango can actually draw', () => {
    assert(toPangoMarkup('# Title').includes('<big><b>Title</b></big>'));
    assert(toPangoMarkup('### Small').includes('<b>Small</b>'));
    assert(toPangoMarkup('- one').includes('• one'));
    assert(toPangoMarkup('* one').includes('• one'));
    assertEq(toPangoMarkup('1. first').trim(), '1. first');
    // A blockquote arrives already escaped as &gt;, which is what the
    // rule has to match — matching '>' would silently never fire.
    assert(toPangoMarkup('> quoted').includes('<i>'), toPangoMarkup('> quoted'));
});

test('only links a person can safely click become links', () => {
    assert(toPangoMarkup('[docs](https://lisaos.dev)')
        .includes('<a href="https://lisaos.dev">docs</a>'));
    // A rendered link is a click target. javascript: and file: in a chat
    // window should not be one keystroke away, so they stay as text.
    const bad = toPangoMarkup('[x](javascript:alert(1))');
    assert(!bad.includes('<a '), bad);
    assert(!bad.includes('javascript:alert(1)') || !bad.includes('href'), bad);
});

test('unbalanced markers never produce unbalanced tags', () => {
    // The important property: whatever a model streams — including a
    // half-written reply mid-stream — must not render as an empty label.
    for (const input of ['**unclosed', '*a', '`code', '```\nblock', '__x', '<b>raw</b>', '&']) {
        const out = toPangoMarkup(input);
        const opens = (out.match(/<(?!\/)[a-z]+/g) ?? []).length;
        const closes = (out.match(/<\/[a-z]+>/g) ?? []).length;
        assertEq(opens, closes, `${JSON.stringify(input)} -> ${out}`);
    }
});

test('empty input is empty, not markup', () => {
    assertEq(toPangoMarkup(''), '');
    assertEq(toPangoMarkup('   '), '');
    assertEq(toPangoMarkup(null), '');
    assertEq(escapeMarkup(undefined), '');
});

finish('assistant/markdown');
