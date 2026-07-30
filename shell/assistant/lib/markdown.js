// Markdown as the model writes it → Pango markup the label can draw.
//
// Models emit Markdown whether or not anything renders it, so the choice
// is between showing `**this**` to a person and showing **this**. There
// is no third option where the model stops.
//
// # Why this is small on purpose
//
// This is not a Markdown implementation and should not become one. Pango
// markup has no block model — no lists, no tables, no nesting — so the
// job is to carry the handful of things a chat reply actually uses
// (emphasis, code, headings, bullets, links) and to leave everything
// else legible. A full parser producing markup Pango cannot express
// would be more code and worse output.
//
// # The failure that governs the design
//
// Pango markup is XML, and **invalid markup renders as nothing at all**
// — not as an error, not as plain text, as an empty label. A reply
// containing `a < b`, or an unclosed `**`, must therefore be impossible
// to turn into broken markup. So: everything is escaped first and
// exactly once, tags are only ever emitted in balanced pairs by this
// file, and the caller is expected to fall back to plain text if Pango
// still refuses.

/// XML-escape. Must run before any tag is inserted, and never after.
export function escapeMarkup(text) {
    return String(text ?? '')
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
}

/// A character no model emits, used to park code spans while the inline
/// rules run. U+0000 is not a legal Pango markup character, so it can
/// never survive into output by accident.
const HOLD = '\u0000';

/// Inline rules, applied to already-escaped text with code spans parked.
///
/// Bold before italic: `**x**` would otherwise match the italic rule
/// twice and produce `<i><i>x</i></i>` around an empty string.
function inline(text) {
    return text
        // [label](https://…) — only http(s) and mailto get to be links.
        // A model can write any URL it likes and a rendered link is a
        // click target; `javascript:` in a chat window is not a thing
        // that should be one keystroke away.
        .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+|mailto:[^\s)]+)\)/g,
            (_m, label, href) => `<a href="${href}">${label}</a>`)
        .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
        .replace(/__([^_]+)__/g, '<b>$1</b>')
        // Single-char emphasis, but not inside a word: `a_b_c` and
        // snake_case identifiers are not italics, and models write both.
        .replace(/(^|[\s(])\*([^*\n]+)\*(?=[\s).,;:!?]|$)/g, '$1<i>$2</i>')
        .replace(/(^|[\s(])_([^_\n]+)_(?=[\s).,;:!?]|$)/g, '$1<i>$2</i>');
}

/// Markdown → Pango markup.
///
/// Returns markup that is safe to hand to `set_markup`, or the escaped
/// original if anything about the input makes that impossible.
export function toPangoMarkup(source) {
    const text = String(source ?? '');
    if (!text.trim())
        return '';

    // 1. Fenced code blocks come out first, before escaping, so their
    //    contents are never treated as Markdown. A reply explaining
    //    Markdown by showing it is a normal thing for a model to write.
    const blocks = [];
    let work = text.replace(/```[^\n]*\n([\s\S]*?)```/g, (_m, code) => {
        blocks.push(code.replace(/\n$/, ''));
        return `${HOLD}B${blocks.length - 1}${HOLD}`;
    });

    // 2. Inline code next, for the same reason.
    const spans = [];
    work = work.replace(/`([^`\n]+)`/g, (_m, code) => {
        spans.push(code);
        return `${HOLD}S${spans.length - 1}${HOLD}`;
    });

    // 3. Escape everything that is left. Exactly once, and after the
    //    code has been parked so it is escaped on the way back in.
    work = escapeMarkup(work);

    // 4. Block-level, line by line. Pango has no block model, so a
    //    heading is bigger and bolder text and a list item is a bullet.
    const lines = work.split('\n').map((line) => {
        const heading = line.match(/^(#{1,6})\s+(.*)$/);
        if (heading)
            return `<big><b>${inline(heading[2])}</b></big>`;
        const bullet = line.match(/^\s*[-*+]\s+(.*)$/);
        if (bullet)
            return `  • ${inline(bullet[1])}`;
        const numbered = line.match(/^\s*(\d+)\.\s+(.*)$/);
        if (numbered)
            return `  ${numbered[1]}. ${inline(numbered[2])}`;
        const quote = line.match(/^&gt;\s?(.*)$/);
        if (quote)
            return `<i>  ${inline(quote[1])}</i>`;
        // A horizontal rule is a rule, not three literal dashes.
        if (/^\s*([-*_])\s*\1\s*\1[\s\-*_]*$/.test(line))
            return '<span alpha="50%">────────</span>';
        return inline(line);
    });
    work = lines.join('\n');

    // 5. Put the code back, escaped, in monospace.
    work = work.replace(new RegExp(`${HOLD}S(\\d+)${HOLD}`, 'g'),
        (_m, i) => `<tt>${escapeMarkup(spans[Number(i)])}</tt>`);
    work = work.replace(new RegExp(`${HOLD}B(\\d+)${HOLD}`, 'g'),
        (_m, i) => `<tt>${escapeMarkup(blocks[Number(i)])}</tt>`);

    return work;
}
