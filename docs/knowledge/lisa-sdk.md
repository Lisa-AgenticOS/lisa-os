<!-- GENERATED into the OS knowledge pack from apps/lisa.sdk/README.md by
     os/repo-tools/build-knowledge.py — edit the source README,
     then regenerate. (#175, ADR-0040) -->

# `lisa.sdk` — the shared GJS library for Lisa surfaces

ADR-0056, ADR-0047 §6. **Steps 1–4 underway**: the widget set grows by
extraction — a widget joins once a real caller needs it, because one
invented up front is a guess. `lisaSplitWindow` was extracted for
Notes; `lisaTripleWindow` for Mail.

| module | what | ADR-0056 |
|---|---|---|
| `mcp/protocol.js` | the JSON-RPC/MCP server edge, and the provenance tag | step 1 |
| `mcp/client.js` | a GJS window talking to its own backend's tools | — |
| `ui/tokens.js` | the generated design tokens, in the payload at last | step 2 |
| `ui/window.js` | `lisaWindow`, `lisaSplitWindow`, `lisaTripleWindow` — one window shape, glass by default | steps 3–4 |
| `ui/style.js` | the Lisa stylesheet, built at runtime from the tokens | step 2 |
| `bus/` | the ONE copy of the seven system D-Bus interfaces + proxy factory | ADR-0060 |

Consumers: `apps/notes` (split window + `mcp/client.js`), `apps/mail`
(triple window — rail+folders | messages | reader), and the overlay
stack (`shell/overlay-extension` serves and consumes `bus/`'s XML).

## `bus/` — the sdk's D-Bus edge (ADR-0060)

`bus/xml/*.xml` is INTROSPECTED from the running daemons on the
reference device — not copied from another hand copy — and
`bus/interfaces.js` is generated from it
(`os/repo-tools/build-bus-interfaces.py`; `--check` runs in the lint
gate). `bus/addresses.js` is the pure name/path table (node-importable,
no `gi://`), and `bus/proxy.js` adds `proxy('Overlay1')` for clients.
The same tool ratchets hand-rolled declarations of the seven out of the
tree: a legacy site can only leave the allowlist, never join it. The
count was ~60 declarations across the GJS surfaces when ADR-0060 was
written; it is 1 (the Assistant's deliberately narrow memory proxy).

## Glass is the default, and it is two halves

A Lisa app with a sidebar gets a see-through pane flush to the window's
left edge without asking. An app opts **out** with `overlay: false`.
That direction is the whole point of a UI library: when looking like
the rest of the system is something each app must remember, you get
#282 — eight surfaces, three answers to where the window controls go.

|  | comes from | needs |
|---|---|---|
| **transparency** | `lisa.sdk` — the window paints nothing behind the pane | any GNOME |
| **frost** | `shell/glass` — the compositor clones the wallpaper, blurs it, slides it under the window | our Shell fork |

Without the extension an app is transparent but **sharp**. That is a
fair degradation, not a broken window — and an app cannot do the frost
half itself at any price, because a client never sees what is behind
its own surface (GNOME/mutter#3023, open).

## What it does

`mcp/protocol.js` is the JSON-RPC/MCP request handler every Lisa app
answers tool calls with, and the one place a result gets its
**provenance tag**. `makeHandler({appId, provenance})` returns the pure
`handleRequest(req, tools)`; the two constants are all an app supplies.

    import {makeHandler} from '../../lisa.sdk/mcp/protocol.js';

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
library, so `../../lisa.sdk/<module>` resolves identically in this repo
and on a device.

## Limits

- **It lives under `apps/`, not `libs/`, and ADR-0056 says `libs`.**
  `build-apps-payload.sh` flattens `apps/<app>` and `shell/<surface>`
  into one tree, so a relative import means the same thing in both
  trees only if the library sits beside the consumer in each. For apps
  that is literal. For `shell/` surfaces it is the `shell/lisa.sdk`
  symlink (→ `../apps/lisa.sdk`, the ADR-0060 pattern in miniature):
  `../lisa.sdk/…` resolves through it in the repo and against the real
  directory at the payload root — verified both sides, and GJS yields
  ONE module instance through a symlink. The symlink is repo plumbing;
  the payload never carries it.
- **The widget set is three window shapes and a button.** Step 4 grows
  by extraction only — the grouped sidebar list that Notes and Mail
  each still hand-roll is the obvious next candidate, when one of them
  changes for a reason the other shares.
- **#282 is not closed.** Notes, Surfer and Mail are on the shared
  chrome — Mail on `lisaTripleWindow`, verified on the device 2026-08-06
  — but the issue's acceptance also names Preview and the Assistant,
  and both still build their own windows.
- **Frost needs `shell/glass`, which is not shipped.** It lives in the
  repo and in `~/.local` on one device. Extensions load from the baked
  tree at session start (#268), so reaching an image means the `lisa`
  package and a release.
- **`Shell.BlurEffect` is not portable.** A GSK cairo fallback cannot
  blur at all, and the frost silently becomes plain transparency. Worth
  a check before anyone promises glass on unknown hardware.
- The library is *not* independently versioned, and there is no API
  version check yet. ADR-0056 requires one before the library and the
  apps can ship on different cadences; today they ship in the same
  payload, so the skew it guards against cannot happen yet.
