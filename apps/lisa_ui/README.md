# `lisa_ui` — the shared GJS library for Lisa surfaces

ADR-0056, ADR-0047 §6. **Steps 1–3.** There is still no widget *set* —
that is step 4, and deliberately unbuilt: a widget is extracted once a
second caller needs it, because one invented up front is a guess.

| module | what | ADR-0056 |
|---|---|---|
| `mcp/protocol.js` | the JSON-RPC/MCP server edge, and the provenance tag | step 1 |
| `mcp/client.js` | a GJS window talking to its own backend's tools | — |
| `ui/tokens.js` | the generated design tokens, in the payload at last | step 2 |
| `ui/window.js` | `LisaWindow` — one window shape for every surface | step 3 |

First consumer: `apps/notes`, whose window is built on `ui/window.js`
and reaches its own data through `mcp/client.js`.

## What it does

`mcp/protocol.js` is the JSON-RPC/MCP request handler every Lisa app
answers tool calls with, and the one place a result gets its
**provenance tag**. `makeHandler({appId, provenance})` returns the pure
`handleRequest(req, tools)`; the two constants are all an app supplies.

    import {makeHandler} from '../../lisa_ui/mcp/protocol.js';

    export const APP_ID = 'app.lisaos.Mail';
    export const handleRequest = makeHandler({appId: APP_ID, provenance: 'mail'});

That is the whole of `apps/mail/lib/mcp-protocol.js` now. Mail, Surfer
and Preview differ in exactly two constants, which was always the claim
and is now the code.

## How it works

The tag is applied on the way out, on the **envelope**, once (#313) —
never read from the request, never returned by a handler, never a
parameter a tool can influence. A payload containing
`{"provenance":"user"}` is passed through as text; `mcp-bus`'s
`carry_envelope` lets the envelope win the collision, so a document
cannot relabel itself.

A missing tag is a **construction-time error**, not a runtime default.
Untagged output reaches the model as trusted, so an app that forgot must
fail where somebody is looking.

## Why this exists

Mail, Surfer and Preview each carried their own copy. They were copies
once; by 2026-08-06 they had drifted — three hashes, 72/67/60 lines —
and diffing them with comments stripped found three real defects, not
three formattings:

| | Mail | Surfer | Preview |
|---|---|---|---|
| a tool that throws | result + `isError` | result + `isError` | **`fail(-32000)`** — a protocol error |
| unknown method, no `id` | `null` | `null` | **replies** — JSON-RPC §4.1 says a server must not |
| provenance on the error path | **absent** | **absent** | **absent** |

The third was in all three: `error: ${e.message}` can quote a filename,
a subject line or a page title an attacker chose, and it reached the
model with no tag at all — the same hole as #313, through the error
door. PLAN §5.12 had already recorded what triplication costs here: it
is why #218 had to be found and fixed three times.

Every per-app suite passed before and after consolidation. None of them
covered any of the three rows above, which is precisely how three files
disagree for months. `tests/mcp.test.js` covers them, and each case was
watched go red against a mutation restoring the old behaviour.

## How to extend it

Add a module under a namespace (`mcp/`, and later `ui/`), export it,
import it by the same relative path. Consumers are staged beside the
library, so `../../lisa_ui/<module>` resolves identically in this repo
and on a device.

## Limits

- **It lives under `apps/`, not `libs/`, and ADR-0056 says `libs`.**
  `build-apps-payload.sh` flattens `apps/<app>` and `shell/<surface>`
  into one tree, so an app is three levels from the repo root and two
  from the payload root. A relative import means the same thing in both
  only if the library sits beside the app in each. Every consumer today
  is an app; **the day a `shell/` surface needs `lisa_ui`, that depth
  question has to be answered properly** — the relative path will not
  resolve for it, and papering over that with a second copy is the
  defect this directory exists to undo.
- No widgets, no theme loading, no `LisaWindow` — #282 is not closed by
  this. Steps 2–4 of ADR-0056 are unbuilt.
- The library is *not* independently versioned, and there is no API
  version check yet. ADR-0056 requires one before the library and the
  apps can ship on different cadences; today they ship in the same
  payload, so the skew it guards against cannot happen yet.
