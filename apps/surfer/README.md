# apps/surfer — Surfer

Spec: `docs/adr/0037-the-browser-is-a-lisa-app.md`, plan issue #146.
GJS + GTK4 + libadwaita + WebKit-6.0 (the engine the image already
ships). App id `app.lisaos.Surfer`.

## What it does

A browser whose current tab the assistant can see. Three read-tier
tools on the Agent Bus — `read_page`, `get_selection`, `screenshot` —
plus three write-tier ones — `navigate`, `click`, `fill` — served over
the MCP socket while a window is open, declared in
`app.lisaos.Surfer.json`. Only the read tier is offered to an agent
loop today (see Limits).

## How it works

- `lisa-surfer.js` — Adw.TabView owns per-tab WebViews; header bar,
  URL entry, Ctrl+T/W/L. **Every window sets `application: app`** — an
  unattached Adw.Window parks WebKit loads at progress 0.1 forever with
  no error anywhere (found in Phase 0, the hard way).
- `lib/url.js` — address-bar → load/search/refused, pure. `javascript:`
  and `data:` are refused everywhere: those execute in the current page.
  `host:port` is not a scheme. `addressBarAction()` is what Enter in the
  bar actually does, including the third outcome — an EMPTY bar loads
  nothing (it used to reach `load_uri(null)`, which throws inside a
  signal handler where nobody sees it, #220) — and it always returns the
  placeholder to show, so a refusal's reason no longer sticks in the
  entry for the rest of the session.
- `lib/world.js` — the named JS world every agent-facing script runs in
  (#212). **`evaluate_javascript`'s third argument is `world_name`, and
  `null` means the page's own world**; there the page owns
  `JSON.stringify` and `document.querySelector`, which is enough to
  forge a whole `read_page` result and to retarget an approved `fill`.
  One non-null world name is the fix. Escaping arguments (below) never
  helped with this: nothing you do to a script's text matters when the
  callee owns the functions it calls.
- `lib/target.js` — which tab a write acts on (#213). A write names its
  page by URL, that URL is an argument so the consent dialog shows it,
  and the action is refused if no open tab is at that address. Nothing
  falls back to "whatever tab is in front of the user" — that fallback
  was the bug.
- `lib/extract.js` — EXTRACT_JS runs in the page's DOM (from the agent
  world) via `evaluate_javascript`; `pageResult()` caps text at 30k
  chars and says `truncated: true` — a truncation the model cannot see
  is a page it thinks it has read.
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
  declared `write` tier in the manifest. What that tier does TODAY, and
  what it does not:
  - `libs/bus-tools`' `read_tier_tools()` offers the model only rows
    with `tier: "read"`, so no agent loop is handed `click` or `fill`
    at all right now. Anything that can open the socket can still call
    them — which is how the bypasses below were reproduced.
  - The consent surface is agentd's, not Surfer's. This README used to
    describe that escalation as behaviour ("agentd escalates them…");
    the end-to-end path — confirmation shown, call in the Ledger — is
    still **not verified on a seated session**, and #216 is open.
  What Surfer itself enforces, all of it deterministic code the model
  cannot reach, and all of it device-verified against the reviewer's
  hostile pages in `/tmp/surfrev/`:
  - `navigate` opens **http: and https: only** (`AGENT_SCHEMES` in
    `lib/actions.js`, #214). The address bar's passthrough list is the
    ADDRESS BAR's rule and still allows `file:` — a person browsing
    their own machine is their business (ADR-0029). Reusing it at the
    agent boundary meant `navigate file:///etc/passwd` + `read_page`
    returned any readable file tagged `provenance: "web"`, around
    contextd's ACLs entirely.
  - `click`/`fill` name the page they act on and are refused if it is
    not open (`lib/target.js`, #213).
  - every agent script runs in the agent world (`lib/world.js`, #212).
  - `lib/actions.js` embeds selectors and values as JSON-escaped data,
    never as script. This was always right and was never the whole
    story: #212 was a layer beneath it.
- **Sidebar tabs (#182 v1)**: vertical tab list in a collapsible left
  sidebar (Ctrl+S), Zen/Arc-shaped; active row carries violet-500.
  Structure and function are device-verified (app runs, journal clean,
  agent tools work through the rebuilt window); **how it LOOKS is
  not** — GNOME 50 refuses remote screenshots, so the first seated
  session judges it. Reorder-by-drag and favicons are follow-ups.
- **MVP**: no bookmarks, downloads, history, passwords, session restore.
- `tests/` cover the pure modules; the window itself is verified by
  eyes on hardware. **A unit test cannot prove world isolation** —
  `tests/world.test.js` pins the `world_name` argument, and the
  isolation itself was proved on the reference device with a page that
  redefines `JSON.stringify` and `document.querySelector`: before the
  fix `read_page` reported the page's forged title and an approved
  `fill` on `#q` landed in `#pw`; after it, the real title and the real
  field. Any future change to how agent scripts are evaluated needs
  that run again, not just a green suite.
