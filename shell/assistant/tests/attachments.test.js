// Unit tests for the Assistant's attachment logic (issue #209's last
// mile): what an attached file becomes on the wire, and the guard that
// stops a local text-only model being handed a picture.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    imageMimeForName, imagePart, attachmentsPayload, attachmentRefusal,
    attachmentSizeRefusal, stagedForSession,
    MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS_TOTAL_BYTES,
} from '../lib/attachments.js';

test('imageMimeForName maps the extensions inferenced forwards', () => {
    assertEq(imageMimeForName('shot.png'), 'image/png');
    assertEq(imageMimeForName('a.JPG'), 'image/jpeg');
    assertEq(imageMimeForName('a.jpeg'), 'image/jpeg');
    assertEq(imageMimeForName('/home/me/pics/x.webp'), 'image/webp');
    assertEq(imageMimeForName('loop.gif'), 'image/gif');
});

test('imageMimeForName refuses what is not an image', () => {
    // null, not a guess: `image/*` on a .pdf produces a provider-side
    // error that reads like our bug.
    assertEq(imageMimeForName('notes.pdf'), null);
    assertEq(imageMimeForName('README'), null);
    assertEq(imageMimeForName(''), null);
    assertEq(imageMimeForName(null), null);
    assertEq(imageMimeForName(42), null);
});

test('imagePart builds the data: URI part the daemons already accept', () => {
    assertEq(imagePart('image/png', 'AAAA'), {
        type: 'image_url',
        image_url: {url: 'data:image/png;base64,AAAA'},
    });
});

test('attachmentsPayload is the parts array, in order, junk dropped', () => {
    const items = [
        {name: 'a.png', mime: 'image/png', b64: 'AAAA'},
        null,
        {name: 'b.gif', mime: 'image/gif', b64: ''},      // empty — dropped
        {name: 'c.webp', mime: '', b64: 'CCCC'},          // no mime — dropped
        {name: 'd.jpg', mime: 'image/jpeg', b64: 'DDDD'},
    ];
    assertEq(attachmentsPayload(items), [
        {type: 'image_url', image_url: {url: 'data:image/png;base64,AAAA'}},
        {type: 'image_url', image_url: {url: 'data:image/jpeg;base64,DDDD'}},
    ]);
    assertEq(attachmentsPayload(null), []);
    assertEq(attachmentsPayload([]), []);
});

test('a local model plus an attachment is refused before it is sent', () => {
    const items = [{name: 'a.png', mime: 'image/png', b64: 'AAAA'}];
    const refusal = attachmentRefusal({id: 'qwen3-0.6b', label: 'qwen3-0.6b'}, items);
    assert(typeof refusal === 'string' && refusal.length > 0,
        'a local model must be refused');
    // It has to name the model and say what to do instead — a refusal
    // that does not is a dead end.
    assert(refusal.includes('qwen3-0.6b'), `names the model: ${refusal}`);
    assert(/cloud/i.test(refusal), `says what would work: ${refusal}`);
});

test('a cloud model may carry attachments', () => {
    const items = [{name: 'a.png', mime: 'image/png', b64: 'AAAA'}];
    assertEq(attachmentRefusal(
        {id: 'remote:anthropic:claude-x', label: 'Anthropic · claude-x'}, items), null);
});

test('no attachments is never a refusal, whatever the model', () => {
    assertEq(attachmentRefusal({id: 'qwen3-0.6b', label: 'qwen'}, []), null);
    assertEq(attachmentRefusal({id: 'qwen3-0.6b', label: 'qwen'}, null), null);
    assertEq(attachmentRefusal(null, []), null);
});

test('an unknown model with an attachment is refused, not waved through', () => {
    // Fail closed: if the picker has no entry we cannot know it is
    // multimodal, and guessing yes is the failure that reaches a person
    // as a confident answer about a picture nobody saw.
    const items = [{name: 'a.png', mime: 'image/png', b64: 'AAAA'}];
    assert(typeof attachmentRefusal(null, items) === 'string');
});

// ---- size (issue #226) --------------------------------------------------
//
// An image over ~1.5 MB came back from the daemons as `413`, after the
// round trip, with nothing a person could act on. Nothing bounded it at
// any layer; the only ceiling was axum's undeclared 2 MiB default.

test('a picture inside the budget is not refused', () => {
    assertEq(attachmentSizeRefusal('shot.png', 1024 * 1024, []), null);
    assertEq(attachmentSizeRefusal('shot.png', MAX_ATTACHMENT_BYTES, []), null);
});

test('one oversized picture is refused at attach time, by name and size', () => {
    const refusal = attachmentSizeRefusal('huge.png', MAX_ATTACHMENT_BYTES + 1, []);
    assert(typeof refusal === 'string' && refusal.length > 0,
        'an oversized image must be refused before it is staged');
    assert(refusal.includes('huge.png'), `names the file: ${refusal}`);
    // The two numbers a person needs to act: how big it is, and how big
    // it may be. "Too large" alone is another round trip.
    assert(/8 MB/.test(refusal), `names the limit: ${refusal}`);
    assert(/MB/.test(refusal.replace('8 MB', '')), `names the size: ${refusal}`);
});

test('the budget is for the whole send, not for each picture', () => {
    // Six 3 MB images are each fine and together are not: the request
    // carries all of them at once.
    const staged = [];
    for (let i = 0; i < 6; i++)
        staged.push({name: `p${i}.png`, bytes: 3 * 1024 * 1024});
    const refusal = attachmentSizeRefusal('p6.png', 3 * 1024 * 1024, staged);
    assert(typeof refusal === 'string' && refusal.length > 0,
        `18 MB already staged plus 3 MB more must be refused: got ${refusal}`);
    assert(/together|altogether|total/i.test(refusal),
        `says it is the total: ${refusal}`);
});

test('the composer budget stays under what harnessd will accept', () => {
    // base64 is 4/3 of the bytes; harnessd refuses an `attachments`
    // option over 24 MiB and inferenced buffers 32 MiB. A composer that
    // allowed more would tell a person "attached" and the daemon would
    // then refuse it — #226 with an extra hop.
    assert(MAX_ATTACHMENTS_TOTAL_BYTES * 4 / 3 < 24 * 1024 * 1024,
        'the composer allows more base64 than harnessd will take');
    assert(MAX_ATTACHMENT_BYTES <= MAX_ATTACHMENTS_TOTAL_BYTES,
        'one picture may not exceed the whole send');
});

test('an attachment belongs to the conversation it was staged in', () => {
    // #235: an image attached in one conversation survived a switch and
    // was sent with the next message — to THAT conversation's provider.
    // Clearing the strip on a switch is the fix; this is the second
    // mechanism, so a switch path nobody remembered to clear still
    // cannot leak the bytes.
    const items = [
        {name: 'salary.png', mime: 'image/png', b64: 'AAAA', session: 's-1'},
        {name: 'ok.png', mime: 'image/png', b64: 'BBBB', session: 's-2'},
        {name: 'untagged.png', mime: 'image/png', b64: 'CCCC'},
    ];
    assertEq(stagedForSession(items, 's-2').map(a => a.name), ['ok.png']);
    assertEq(stagedForSession(items, 's-1').map(a => a.name), ['salary.png']);
    // An attachment with no session is nobody's, not everybody's.
    assertEq(stagedForSession(items, undefined), []);
    assertEq(stagedForSession(null, 's-1'), []);
});

test('the payload never carries another conversation attachment', () => {
    const items = [
        {name: 'salary.png', mime: 'image/png', b64: 'AAAA', session: 's-1'},
        {name: 'ok.png', mime: 'image/png', b64: 'BBBB', session: 's-2'},
    ];
    assertEq(attachmentsPayload(stagedForSession(items, 's-2')), [
        {type: 'image_url', image_url: {url: 'data:image/png;base64,BBBB'}},
    ]);
});

await finish('assistant attachments');
