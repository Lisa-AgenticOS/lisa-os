// Mail's MCP surface: the shared protocol, plus the two constants that
// are genuinely Mail's (ADR-0056 step 1).
//
// Mail is the most consequential context source Lisa has. A web page is
// something you chose to open; a message is something anyone can send
// you, and its entire text is attacker-controlled by construction. That
// is what the `mail` tag is for: agentd escalates the confirmation tier
// of any privileged call whose chain includes it (PLAN §5.10, Appendix
// C), so "summarise my mail and then delete something" asks before it
// deletes.
//
// The protocol itself used to live here, in a copy that had drifted from
// Surfer's and Preview's — see apps/lisa_ui/mcp/protocol.js for what the
// drift turned out to be.

import {makeHandler} from '../../lisa_ui/mcp/protocol.js';

export const APP_ID = 'app.lisaos.Mail';

/// Everything this app emits is mail-provenance. Not a parameter, not
/// read from the message, not overridable by a handler: a constant,
/// applied on the way out.
const PROVENANCE = 'mail';

export const handleRequest = makeHandler({appId: APP_ID, provenance: PROVENANCE});
