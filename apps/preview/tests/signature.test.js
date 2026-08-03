import {strokeBounds, normalizeStrokes, stampSize, serializeSignature, deserializeSignature}
    from '../lib/signature.js';

let passed = 0, failed = 0;
function ok(cond, name, got) {
    if (cond) { passed++; console.log(`  ok    ${name}`); }
    else { failed++; console.log(`  FAIL  ${name}${got !== undefined ? ` (got ${JSON.stringify(got)})` : ''}`); }
}

const scrawl = [
    [{x: 10, y: 20}, {x: 30, y: 40}],
    [{x: 5, y: 25}, {x: 50, y: 30}],
];
const b = strokeBounds(scrawl);
ok(b.x1 === 5 && b.y1 === 20 && b.x2 === 50 && b.y2 === 40, 'bounds span all strokes', b);
ok(strokeBounds([]) === null, 'no ink, no bounds');
ok(strokeBounds([[]]) === null, 'an empty stroke is not ink either');

const sig = normalizeStrokes(scrawl);
ok(sig.width === 45 + 12 && sig.height === 20 + 12, 'normalized size is the bbox plus padding', [sig.width, sig.height]);
ok(sig.strokes[1][0].x === 6 && sig.strokes[0][0].y === 6, 'strokes translate to the padded origin');
ok(normalizeStrokes([]) === null, 'saving an empty canvas is a no-op, not a 0×0 stamp');

const size = stampSize({width: 300, height: 100});
ok(size.width === 150 && size.height === 50, 'the stamp keeps the signature aspect at the target width', size);
const dot = stampSize({width: 2, height: 400});
ok(dot.height <= 150, 'a degenerate tall scrawl is clamped, never a page-height stamp', dot);
ok(stampSize({width: 300, height: 1}).height >= 8, 'a flat scrawl still gets a visible height');

const round = deserializeSignature(serializeSignature(sig));
ok(round.width === sig.width && round.strokes.length === 2, 'serialize/deserialize round-trips');
ok(deserializeSignature('not json') === null, 'garbage input is null, not a throw');
ok(deserializeSignature('{"version":99,"strokes":[]}') === null, 'unknown versions are refused');

console.log(`preview/signature: ${passed} passed, ${failed} failed`);
if (failed) process.exit(1);
