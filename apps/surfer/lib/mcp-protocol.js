// Surfer's MCP surface: the shared protocol, plus the two constants that
// are genuinely Surfer's (#146 Phase 3, ADR-0056 step 1).
//
// `web` is the original untrusted tag and the one the injection suite is
// built around: a page is content the person chose to open, but its text
// is written by whoever wrote the page. agentd escalates the
// confirmation tier of any privileged call whose chain carries it.
//
// The protocol itself used to live here. Surfer's copy was the one that
// inlined the tag as a bare string rather than naming it, and the one
// whose tag was lost entirely on its first on-device run (#313) — see
// apps/lisa_ui/mcp/protocol.js.

import {makeHandler} from '../../lisa_ui/mcp/protocol.js';

export const APP_ID = 'app.lisaos.Surfer';

/// Everything this app emits is web-provenance. Named, not inlined at
/// the return site: a constant with a name is a constant somebody can
/// grep for.
const PROVENANCE = 'web';

export const handleRequest = makeHandler({appId: APP_ID, provenance: PROVENANCE});
