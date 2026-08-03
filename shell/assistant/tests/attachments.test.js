// Unit tests for the Assistant's attachment logic (issue #209's last
// mile): what an attached file becomes on the wire, and the guard that
// stops a local text-only model being handed a picture.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    imageMimeForName, imagePart, attachmentsPayload, attachmentRefusal,
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

await finish('assistant attachments');
