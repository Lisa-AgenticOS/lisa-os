// The Write-tier Agent Bus tools, as decisions rather than effects
// (issue #167). Pure: every function here says what should happen, and
// lisa-mail.js performs it with the same `applyMove` the toolbar uses.
//
// ONE IMPLEMENTATION, TWO CALLERS. The buttons already resolve to
// `flagChange`/`moveTo` in lib/actions.js; these tools resolve to the
// same two functions rather than reimplementing "what does archive
// mean". A second implementation of a Maildir rename is a second place
// to get the `new/`→`cur/` promotion wrong, and the two would drift in
// exactly the way that is invisible until a message ends up in neither
// folder.
//
// # Why these are Write tier
//
// Mail is untrusted input the agent reads and then acts on — the shape
// ADR-0029 and PLAN §5.10 exist for. A message whose body says "archive
// everything from the bank" is a message, not an instruction. Tagging
// tool results `mail` provenance (lib/mcp-protocol.js) makes agentd
// escalate a Write call that follows a read, so the confirmation names
// the real requester. That machinery already works; these tools are
// what finally exercise it.

import {flagChange, moveTo} from './actions.js';

/// `"INBOX/1785515977.25479_1.lisa_U_8401"` → `{folder, unique}`.
///
/// Same shape `search_mail` emits and `read_message` accepts, so an
/// agent can pass an id straight from one tool to the next. Splitting
/// on the FIRST slash only: the remainder is the filename, and
/// `messagePath` refuses a filename containing a slash, so a nested
/// path cannot be smuggled through here.
export function parseMessageId(id) {
    const text = String(id ?? '');
    const slash = text.indexOf('/');
    if (slash <= 0 || slash === text.length - 1)
        return {error: 'id must be "<folder>/<message>" as returned by search_mail'};
    return {folder: text.slice(0, slash), unique: text.slice(slash + 1)};
}

/// What a tool call should do to a message.
///
/// Returns `{kind:'flag'|'move', change, toFolder}` for the performer,
/// or `{error}` — never throws, and never a silent no-op: "already
/// archived" is an answer, and a tool that returns success having done
/// nothing is the failure mode this repo keeps finding.
export function planFor(tool, message, args = {}, folders = []) {
    if (!message)
        return {error: 'no such message'};

    switch (tool) {
    case 'flag_message': {
        const on = args.flagged !== false;
        const change = flagChange(message, 'F', on);
        if (!change)
            return {noop: `message is already ${on ? 'pinned' : 'unpinned'}`};
        return {kind: 'flag', change, toFolder: message.folder};
    }
    case 'mark_read': {
        const on = args.read !== false;
        const change = flagChange(message, 'S', on);
        if (!change)
            return {noop: `message is already marked ${on ? 'read' : 'unread'}`};
        return {kind: 'flag', change, toFolder: message.folder};
    }
    case 'archive_message':
        return moveInto(message, 'Archive', folders);
    case 'trash_message':
        return moveInto(message, 'Trash', folders);
    case 'move_message': {
        const target = String(args.folder ?? '').trim();
        if (!target)
            return {error: 'move_message needs a folder'};
        return moveInto(message, target, folders);
    }
    default:
        return {error: `no tool ${JSON.stringify(tool)}`};
    }
}

function moveInto(message, folder, folders) {
    // A folder that does not exist is an error, not a folder we create.
    // Inventing "Archive" on a synced Maildir makes a folder the server
    // has never heard of, and the next sync either deletes it or
    // propagates it — neither is what the caller asked for.
    if (!folders.includes(folder))
        return {error: `no folder ${JSON.stringify(folder)} in this maildir`};
    const change = moveTo(message, folder);
    if (!change)
        return {noop: `message is already in ${folder}`};
    return {kind: 'move', change, toFolder: change.toFolder};
}
