# mcp-bus — MCP transport & registry library

Spec: docs/PLAN.md §5.4. Milestone: M5.

Vendored/wrapped MCP SDK: per-app unix socket transport, manifest schema (docs/specs/app-manifest.md), activation semantics. Shared by agentd, the portal, and app-side helpers.

Status: **dispatcher transport landed (ADR-0013)** — `McpClient` (newline-delimited JSON-RPC 2.0 over unix sockets: `initialize` → `notifications/initialized` → `tools/call`) and `McpDispatcher` (agentd `Dispatcher` shape, socket dir default `/run/lisa/mcp`, per-op timeout). Manifest schema and registry still live in `daemons/agentd`; socket activation (`mcp.activatable`) is deferred. Manifest schema/registry extraction and app-side helpers remain TODO.

## How a tool result is shaped, and why the envelope survives (#313)

`extract_tool_result` turns an MCP `tools/call` result into the single
`Value` the Agent Bus journals and hands the model:

| The app replies with | The bus sees |
|---|---|
| `isError: true` | `Err(McpError::Tool(<text>))` |
| `structuredContent` | that value |
| one `text` block holding JSON | the parsed value |
| one `text` block of prose | that string |
| anything else | the result verbatim |

In every unwrapping case the envelope's **own** fields — everything
beside `content` / `structuredContent` / `isError` — are hoisted onto
the value that comes back (`carry_envelope`). The field that matters is
`provenance`, the tag that decides whether the run gets tainted:

```rust
// the app's reply                          // what the loop reads
{ "content": [{"type":"text",              { "title": "T",
               "text": "{\"title\":\"T\"}"}],  "provenance": "web" }
  "provenance": "web" }
```

Three rules, each in the safe direction:

- **The envelope wins a collision.** It is written by the app's own edge
  — a constant in `lib/mcp-protocol.js` — while the payload is
  assembled from whatever the app just read. A mail body reading
  `{"provenance":"user"}` is text, not a claim.
- **A payload that cannot hold siblings is wrapped** under `content`, so
  a lone array or string does not silently drop the tag.
- **An envelope carrying nothing but shape keys is passed through
  unchanged**, so the common `{content: […]}` reply costs nothing.

Before this, the unwrap discarded the envelope, and the tag survived
only because all three shipped apps wrote it **twice** — once on the
envelope and once inside the payload — each with a comment explaining
the workaround. Three copies of a workaround for one bug is not a
mechanism: the fourth app tags the envelope the way MCP invites, is
discarded, and is treated as trusted, which is #302's failure mode
through a different door. The double-tagging is gone from the apps as of
this change; the tag has one home.

`libs/mcp-bus/tests/envelope_provenance.rs` drives a real MCP server
over a real socket and reads the result with
`bus_tools::untrusted_result_provenance` — the function the agent loop
actually uses — so the assertion is about the product, not about a
re-implementation of it.

## Limits

- **The error path carries no provenance.** `isError: true` becomes
  `McpError::Tool(text)`; agentd reports that as a `failed` disposition
  and `untrusted_result_provenance` only reads the `executed` one. So an
  app can put page text in front of the model inside an *error message*
  and it costs the run nothing. No shipped app does — Surfer and Mail
  return `error: <message>` and Preview returns a JSON-RPC error — but
  nothing stops one, and the fix belongs on the disposition side rather
  than here.
- **Socket activation (`mcp.activatable`) is not implemented.** The
  app's socket must already be live; otherwise the dispatch fails
  cleanly and the bus ledgers it as failed.
- **The manifest schema and registry still live in `daemons/agentd`.**
