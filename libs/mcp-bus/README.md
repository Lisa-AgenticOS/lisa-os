# mcp-bus — MCP transport & registry library

Spec: docs/PLAN.md §5.4. Milestone: M5.

Vendored/wrapped MCP SDK: per-app unix socket transport, manifest schema (docs/specs/app-manifest.md), activation semantics. Shared by agentd, the portal, and app-side helpers.

Status: **dispatcher transport landed (ADR-0013)** — `McpClient` (newline-delimited JSON-RPC 2.0 over unix sockets: `initialize` → `notifications/initialized` → `tools/call`) and `McpDispatcher` (agentd `Dispatcher` shape, socket dir default `/run/lisa/mcp`, per-op timeout). Manifest schema and registry still live in `daemons/agentd`; socket activation (`mcp.activatable`) is deferred. Manifest schema/registry extraction and app-side helpers remain TODO.
