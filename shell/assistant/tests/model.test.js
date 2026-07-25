// Unit tests for the Lisa Assistant view-model (the chat window's ADR;
// PLAN §5.7.1): model-list assembly (local + cloud), the send payload,
// and the egress marker.
import {test, assert, assertEq, finish} from '../../testing/harness.js';
import {
    isRemote, parseLocalModels, usableProviders, cloudEntries,
    mergeModelList, historyPayload, conversationMarkdown,
    normalizeTurns, deserializeConversation,
} from '../lib/model.js';

test('parseLocalModels reads /v1/models data ids', () => {
    const json = {object: 'list', data: [{id: 'qwen3-0.6b'}, {id: 'phi'}]};
    assertEq(parseLocalModels(json),
        [{id: 'qwen3-0.6b', label: 'qwen3-0.6b', kind: 'local'},
         {id: 'phi', label: 'phi', kind: 'local'}]);
});

test('parseLocalModels tolerates junk', () => {
    assertEq(parseLocalModels(null), []);
    assertEq(parseLocalModels({data: [{}, {id: ''}, {id: 'ok'}]}),
        [{id: 'ok', label: 'ok', kind: 'local'}]);
});

test('usableProviders keeps only signed-in or keyed providers', () => {
    const state = {providers: [
        {id: 'anthropic', display_name: 'Anthropic', connected: true, has_key: false},
        {id: 'openai', display_name: 'OpenAI', connected: false, has_key: true},
        {id: 'together', display_name: 'Together', connected: false, has_key: false},
    ]};
    assertEq(usableProviders(state), [
        {id: 'anthropic', display_name: 'Anthropic'},
        {id: 'openai', display_name: 'OpenAI'},
    ]);
});

test('cloudEntries builds routable remote ids and labels', () => {
    assertEq(cloudEntries('anthropic', 'Anthropic', ['claude-x', 'claude-y']), [
        {id: 'remote:anthropic:claude-x', label: 'Anthropic · claude-x',
            kind: 'cloud', provider: 'anthropic'},
        {id: 'remote:anthropic:claude-y', label: 'Anthropic · claude-y',
            kind: 'cloud', provider: 'anthropic'},
    ]);
});

test('mergeModelList keeps local first and dedupes by id', () => {
    const local = [{id: 'qwen', label: 'qwen', kind: 'local'}];
    const cloud = [
        {id: 'remote:openai:gpt', label: 'OpenAI · gpt', kind: 'cloud'},
        {id: 'remote:openai:gpt', label: 'dup', kind: 'cloud'},
    ];
    const merged = mergeModelList(local, cloud);
    assertEq(merged.map(m => m.id), ['qwen', 'remote:openai:gpt']);
});

test('historyPayload maps completed turns to messages', () => {
    const turns = [
        {role: 'user', text: 'hi'},
        {role: 'assistant', text: 'hello'},
        {role: 'assistant', text: ''},   // in-flight/empty — dropped
    ];
    assertEq(historyPayload(turns), [
        {role: 'user', content: 'hi'},
        {role: 'assistant', content: 'hello'},
    ]);
});

test('isRemote flags broker-routed ids', () => {
    assert(isRemote('remote:anthropic:claude'), 'remote: is remote');
    assert(!isRemote('qwen3-0.6b'), 'local is not remote');
});

// ---- conversationMarkdown (issue #8) --------------------------------

const MODELS = [
    {id: 'qwen', label: 'qwen', kind: 'local'},
    {id: 'remote:anthropic:claude-x', label: 'Anthropic · claude-x',
        kind: 'cloud', provider: 'anthropic'},
];

test('conversationMarkdown renders an empty conversation as empty', () => {
    assertEq(conversationMarkdown([], MODELS), '');
    assertEq(conversationMarkdown(null, MODELS), '');
    assertEq(conversationMarkdown([{role: 'assistant', text: ''}], MODELS), '');
});

test('conversationMarkdown renders a local exchange', () => {
    const turns = [
        {role: 'user', text: 'hi'},
        {role: 'assistant', text: 'hello', model: 'qwen'},
    ];
    assertEq(conversationMarkdown(turns, MODELS),
        '**You:**\n\nhi\n\n**Lisa (qwen):**\n\nhello\n');
});

test('conversationMarkdown notes egress on cloud turns', () => {
    const turns = [
        {role: 'user', text: 'q'},
        {role: 'assistant', text: 'a', model: 'remote:anthropic:claude-x'},
    ];
    assertEq(conversationMarkdown(turns, MODELS),
        '**You:**\n\nq\n\n' +
        '**Lisa (Anthropic · claude-x):**\n\na\n\n' +
        '_↗ left this machine via Anthropic · claude-x_\n');
});

test('conversationMarkdown preserves multiline text', () => {
    const turns = [{role: 'assistant', text: 'one\ntwo\n\nthree', model: null}];
    assertEq(conversationMarkdown(turns, MODELS),
        '**Lisa:**\n\none\ntwo\n\nthree\n');
});

// ---- Context1 persistence helpers -----------------------------------

test('normalizeTurns keeps only completed turns, no widgets', () => {
    const turns = [
        {role: 'user', text: 'hi', widget: {}, body: {}},
        {role: 'assistant', text: 'hello', model: 'qwen', widget: {}},
        {role: 'assistant', text: ''},   // in-flight — dropped
    ];
    assertEq(normalizeTurns(turns), [
        {role: 'user', text: 'hi', model: null},
        {role: 'assistant', text: 'hello', model: 'qwen'},
    ]);
});

test('deserializeConversation validates shape and drops junk', () => {
    assertEq(deserializeConversation('not json'), []);
    assertEq(deserializeConversation('{"role":"user"}'), []);
    assertEq(deserializeConversation('[]'), []);
    assertEq(deserializeConversation(JSON.stringify([
        {role: 'user', text: 'ok'},
        {role: 'tool', text: 'nope'},
        {role: 'assistant', text: 42},
        {role: 'assistant', text: 'fine', model: 7},
        null,
    ])), [
        {role: 'user', text: 'ok', model: null},
        {role: 'assistant', text: 'fine', model: null},
    ]);
});

test('conversation persistence round-trips', () => {
    const turns = [
        {role: 'user', text: 'q', model: null},
        {role: 'assistant', text: 'a', model: 'remote:anthropic:claude-x'},
    ];
    assertEq(deserializeConversation(JSON.stringify(normalizeTurns(turns))),
        turns);
});

finish('assistant-model');
