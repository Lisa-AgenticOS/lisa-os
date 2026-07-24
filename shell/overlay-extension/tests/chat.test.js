// Unit tests for the assistant chat-lane helpers (PLAN §5.7.1; the chat
// window's ADR): multi-turn message assembly, request-body shaping, and
// OpenAI SSE parsing.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    buildMessages, chatRequestBody, parseSseLine, isRemoteModel,
} from '../lib/chat.js';

test('buildMessages appends the prompt as the final user turn', () => {
    const history = [
        {role: 'user', content: 'hi'},
        {role: 'assistant', content: 'hello'},
    ];
    assertEq(buildMessages(history, 'how are you?'), [
        {role: 'user', content: 'hi'},
        {role: 'assistant', content: 'hello'},
        {role: 'user', content: 'how are you?'},
    ]);
});

test('buildMessages drops malformed / empty history turns', () => {
    const history = [
        {role: 'system', content: 'x'},   // wrong role
        {role: 'user', content: ''},      // empty
        {role: 'user'},                   // no content
        {role: 'assistant', content: 'kept'},
        null,
    ];
    assertEq(buildMessages(history, 'q'), [
        {role: 'assistant', content: 'kept'},
        {role: 'user', content: 'q'},
    ]);
});

test('buildMessages tolerates no history', () => {
    assertEq(buildMessages(undefined, 'q'), [{role: 'user', content: 'q'}]);
});

test('chatRequestBody streams and includes the model when set', () => {
    assertEq(chatRequestBody('remote:anthropic:claude', [{role: 'user', content: 'q'}]),
        {messages: [{role: 'user', content: 'q'}], stream: true,
            model: 'remote:anthropic:claude'});
});

test('chatRequestBody omits an unset model', () => {
    const body = chatRequestBody(undefined, []);
    assert(!('model' in body), 'no model key when unset');
    assertEq(body.stream, true);
});

test('parseSseLine extracts a content delta', () => {
    const line = 'data: {"choices":[{"delta":{"content":"Hel"}}]}';
    assertEq(parseSseLine(line), {delta: 'Hel'});
});

test('parseSseLine recognises the terminator', () => {
    assertEq(parseSseLine('data: [DONE]'), {done: true});
});

test('parseSseLine surfaces a mid-stream error', () => {
    assertEq(parseSseLine('data: {"error":{"message":"boom"}}'), {error: 'boom'});
});

test('parseSseLine ignores blanks, comments and role-only deltas', () => {
    assertEq(parseSseLine(''), null);
    assertEq(parseSseLine(': keep-alive'), null);
    assertEq(parseSseLine('data: {"choices":[{"delta":{"role":"assistant"}}]}'), null);
    assertEq(parseSseLine('data: not json'), null);
});

test('isRemoteModel detects the broker route', () => {
    assert(isRemoteModel('remote:openai:gpt-4o'), 'remote: is remote');
    assert(!isRemoteModel('qwen3-0.6b'), 'local id is not remote');
    assert(!isRemoteModel(undefined), 'undefined is not remote');
});

finish('chat');
