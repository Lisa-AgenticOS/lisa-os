import {showFileAction, BUS_NAME, OBJECT_PATH} from '../lib/previewer-protocol.js';

let pass = 0, fail = 0;
function ok(c, w) { if (c) { pass++; console.log(`  ok    ${w}`); } else { fail++; console.log(`  FAIL  ${w}`); } }
const eq = (a, b, w) => ok(JSON.stringify(a) === JSON.stringify(b), `${w} (got ${JSON.stringify(a)})`);

console.log('preview/previewer');

// Quick Look's rule: Space on what you are already looking at closes it.
eq(showFileAction('file:///a.png', {uri: 'file:///a.png', visible: true}, true),
   {action: 'close'}, 'space on the file already shown closes it');

// The one that is annoying to get wrong: Space on a DIFFERENT file must
// swap, not close. Closing there fights the user mid-browse.
eq(showFileAction('file:///b.png', {uri: 'file:///a.png', visible: true}, true),
   {action: 'show', uri: 'file:///b.png'}, 'space on a different file swaps to it');

eq(showFileAction('file:///a.png', {uri: 'file:///a.png', visible: false}, true),
   {action: 'show', uri: 'file:///a.png'}, 'the same file, not currently visible, opens');

eq(showFileAction('file:///a.png', {uri: 'file:///a.png', visible: true}, false),
   {action: 'show', uri: 'file:///a.png'}, 'the caller can ask for show-without-toggle');

// The names Nautilus actually calls. Wrong here = the key silently does
// nothing, which is indistinguishable from the feature not existing.
// Nautilus release builds append an empty PROFILE to both: the name and
// path are versionless; only the INTERFACE is NautilusPreviewer2
// (nautilus 50.2.2 src/nautilus-previewer.c:43-44). The "2" name was
// shipped once and Space died silently — Nautilus ping-gates it.
ok(BUS_NAME === 'org.gnome.NautilusPreviewer', 'the bus name is versionless — Nautilus never dials the 2');
ok(OBJECT_PATH === '/org/gnome/NautilusPreviewer', 'the object path is versionless too');

console.log(`preview/previewer: ${pass} passed, ${fail} failed`);
if (fail) throw new Error(`${fail} failed`);
