<!-- GENERATED into the OS knowledge pack from apps/surfer/README.md by
     os/repo-tools/build-knowledge.py — edit the source README,
     then regenerate. (#175, ADR-0040) -->

# apps/surfer — Surfer

Spec: `docs/adr/0037-the-browser-is-a-lisa-app.md`, plan issue #146.
GJS + GTK4 + libadwaita + WebKit-6.0 (the engine the image already
ships). App id `app.lisaos.Surfer`.

## What it does

A browser whose current tab the assistant can see. Three read-tier
tools on the Agent Bus — `read_page`, `get_selection`, `screenshot` —
served over the MCP socket while a window is open, declared in
`app.lisaos.Surfer.json`.

## How it works

- `lisa-surfer.js` — Adw.TabView owns per-tab WebViews; header bar,
  URL entry, Ctrl+T/W/L. **Every window sets `application: app`** — an
  unattached Adw.Window parks WebKit loads at progress 0.1 forever with
  no error anywhere (found in Phase 0, the hard way).
- `lib/url.js` — address-bar → load/search/refused, pure. `javascript:`
  and `data:` are refused: `navigate` will be a bus tool, and those
  execute in the current page. `host:port` is not a scheme.
- `lib/extract.js` — EXTRACT_JS runs in the page via
  `evaluate_javascript`; `pageResult()` caps text at 30k chars and says
  `truncated: true` — a truncation the model cannot see is a page it
  thinks it has read.
- `lib/mcp-protocol.js` — the JSON-RPC surface, pure. **Every
  `tools/call` result carries `provenance: "web"`** (ADR-0037 §2): the
  tag goes on at the edge where page content leaves the browser. The
  harness (`cli/lisa/src/bus_tools.rs`) reads it and taints the chain,
  so a later Write-tier call escalates (Provenance::Web is untrusted in
  agentd's `resolve()`).
- `lib/mcp.js` — the socket, `$XDG_RUNTIME_DIR/lisa/mcp/<app>.sock`,
  newline JSON-RPC per `libs/mcp-bus`. Unlinked on exit.

## Limits

- **Tools exist only while a window is open** — mcp-bus defers socket
  activation, deliberately.
- **No Widevine** (no Netflix/Spotify), **no WebExtensions** (no uBlock).
  Accepted in ADR-0037 §3; Zen stays one `lisa apps` command away.
- **Write tools (`navigate`, `click`, `fill`) exist** (#166) and are
  declared `write` tier in the manifest: agentd escalates them through
  the consent surface, and because page content is `web` provenance
  (untrusted), any write following a `read_page` escalates by
  construction. `navigate` goes through `lib/url.js`'s one refusal
  list (no `javascript:`/`data:`/`vbscript:`/`blob:`); `lib/actions.js`
  embeds selectors and values as JSON-escaped data, never as script.
  **Not yet verified on the device**: the end-to-end acceptance (a
  real form filled with the confirmation appearing in the consent
  surface and the calls in the Ledger) needs the seated session.
- **Sidebar tabs (#182 v1)**: vertical tab list in a collapsible left
  sidebar (Ctrl+S), Zen/Arc-shaped; active row carries violet-500.
  Structure and function are device-verified (app runs, journal clean,
  agent tools work through the rebuilt window); **how it LOOKS is
  not** — GNOME 50 refuses remote screenshots, so the first seated
  session judges it. Reorder-by-drag and favicons are follow-ups.
- **MVP**: no bookmarks, downloads, history, passwords, session restore.
- `tests/` cover the pure modules; the window itself is verified by
  eyes on hardware.
