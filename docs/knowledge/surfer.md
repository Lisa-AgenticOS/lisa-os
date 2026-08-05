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
plus three write-tier ones — `navigate`, `click`, `fill` — served over
the MCP socket while a window is open, declared in
`app.lisaos.Surfer.json`. Only the read tier is offered to an agent
loop today (see Limits).

It is also an ordinary browser, which it has to be to be the only one
on the image: tabs in a sidebar, **downloads** with a progress list and
a conflict dialog, **find in page** (Ctrl+F), **history** (Ctrl+H) that
is searchable and deletable, **bookmarks** (Ctrl+D), **session restore**
that can be switched off, **zoom** (Ctrl +/-/0) and **print** (Ctrl+P).
All of it is per profile: `lib/store.js` is the only place a profile
name becomes a path.

### Keys

| | |
|---|---|
| Ctrl+T / Ctrl+W / Ctrl+L | new tab, close tab, focus the address bar |
| Ctrl+S | collapse the sidebar to a rail |
| Ctrl+F, Ctrl+G, Ctrl+Shift+G | find in page, next match, previous |
| Ctrl+D | bookmark this page (and un-bookmark it) |
| Ctrl+H, Ctrl+Shift+O, Ctrl+J | history, bookmarks, downloads |
| Ctrl+plus / Ctrl+minus / Ctrl+0 | zoom, per tab |
| Ctrl+P | print |

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
- `lib/store.js` — **the only place a profile name becomes a path.**
  History, bookmarks, downloads, the session snapshot and the one
  setting all go through `profileStorePath(profile, base, kind)`, and
  `kind` is an allowlist rather than a sanitised string. `personal` keeps
  the base directory it already had; everything else is under
  `profiles/<name>/`, so the agent profile's history cannot be appended
  to the person's. A corrupt or truncated file decodes as an empty list —
  losing a history is bad and being unable to start is worse.
- `lib/downloads.js` — a download is a **write to disk driven by a remote
  server**, so all three of its decisions are here and tested:
  `safeFilename` reduces a `Content-Disposition` name to one path
  component (`../../.config/autostart/x.desktop` becomes
  `configautostartx.desktop`, and the URI fallback is decoded BEFORE the
  separators are stripped, because `%2F` decodes to one);
  `destinationFor` **never returns `save` for a path that exists**; and
  `resolveConflict` treats every answer it does not recognise — Escape,
  the close button, a stale value — as a cancel.
- `lib/find.js` — the `WebKitFindOptions` bits, pinned against the GIR on
  the reference device, plus the counter's three states. "Searching…" is
  a state: a count that has not come back yet must not render as zero.
- `lib/history.js` — one row per URL with a visit count, newest first.
  `recordable(url, profile)` takes the profile and **refuses the agent
  profile**; the three forget functions (`forgetUrl`, `forgetSince`,
  `clearHistory`) delete every matching row rather than the first one.
- `lib/bookmarks.js` — add/remove/search, deduplicated by URL, with an
  allowlist of schemes: a stored `javascript:` row is a self-XSS with a
  nice icon, which is why Firefox and Chrome both stopped honouring them.
- `lib/session.js` — the snapshot and the switch. Restore is on by
  default and only an exact `false` turns it off, so a half-written
  settings file cannot silently disable it; turning it off **deletes the
  snapshot** rather than merely declining to read it.
- `lib/zoom.js` — the step list, because `level * 1.1` compounds until
  Ctrl+0 no longer lands exactly on 100%.

### The pieces that are only in the window

- **`download-started` is on `WebKit.NetworkSession`, not on the
  WebView.** The 6.0 API moved it (`WebKitNetworkSession.cpp:203` in the
  GIR); wiring it per view would also miss downloads from popups and
  redirects. It is connected once, where the session is built.
- **A response WebKit cannot render is a file, not a blank page.**
  `decide-policy` on `PolicyDecisionType.RESPONSE` calls
  `decision.download()` when `is_mime_type_supported()` is false. Without
  it, clicking a `.zip` opens an empty tab and nothing says why: WebKit
  only starts a download by itself for `Content-Disposition: attachment`,
  which most servers do not send.
- **A conflict is answered asynchronously.** `decide-destination` returns
  `true` *without* setting a destination, which since WebKitGTK 2.40
  means "wait for me"; the transfer does not proceed until
  `set_destination` or `cancel`. So an unanswered dialog is a stalled
  download, never an overwritten file.
- **The session is written from `release()`, not from `close-request`.**
  Every path that ends the process goes through `release()` — a window
  close, SIGTERM, a logout that stops the session's units — and a session
  that survives only a polite quit is a restore nobody can rely on.

Smallest real example, which is also how this was verified on hardware:

```
$ LISA_SURFER_DOWNLOAD_DIR=/tmp/surfrev/dl XDG_DATA_HOME=/tmp/surfrev/data \
    gjs -m apps/surfer/lisa-surfer.js http://127.0.0.1:8899/hello.bin
$ cat /tmp/surfrev/dl/hello.bin
lisa-surfer download probe payload
$ cat /tmp/surfrev/data/lisa-surfer/downloads.json
{"surfer_store":1,"downloads":[{"id":"d1","uri":"http://127.0.0.1:8899/hello.bin",
 "filename":"hello.bin","path":"/tmp/surfrev/dl/hello.bin","state":"done",…}]}
```

`LISA_SURFER_DOWNLOAD_DIR` exists so the download path can be exercised
on a real machine without writing into somebody's actual Downloads
folder — the same reason `LISA_SURFER_UA` exists.

## How to extend it

A new browser feature is a new `lib/*.js` with a matching
`tests/*.test.js`, and only the widget in `lisa-surfer.js`. Concretely:

- **Something that persists** goes through `lib/store.js`: add its file
  to `STORE_FILES` and a `storeLoad`/`storeSave` pair. Do not build a
  path anywhere else — that is the one rule this app has about storage,
  and `tests/store.test.js` asserts no two profiles can ever collide.
- **Something an agent could reach** needs the question asked out loud:
  can a page cause it, and can `navigate`/`click` cause it? The download
  path is the worked example — see the agent boundary below.
- **A WebKit enum you copy into a pure module** needs a test naming the
  GIR it was copied from (`tests/find.test.js` does this), because a
  copy with no test is a copy that drifts.

## Limits

- **Tools exist only while a window is open** — mcp-bus defers socket
  activation, deliberately.
- **No Widevine** (no Netflix/Spotify), **no WebExtensions** (no uBlock).
  Accepted in ADR-0037 §3. The escape hatch named there was Zen on the
  apps channel; that channel was retired on 2026-08-05, so today the
  answer is installing another browser from Arch yourself.
- **Write tools (`navigate`, `click`, `fill`) exist** (#166) and are
  declared `write` tier in the manifest. What that tier does TODAY, and
  what it does not:
  - `libs/bus-tools`' `read_tier_tools()` offers the model only rows
    with `tier: "read"`, so no agent loop is handed `click` or `fill`
    at all right now. Anything that can open the socket can still call
    them — which is how the bypasses below were reproduced.
  - The consent surface is agentd's, not Surfer's. This README used to
    describe that escalation as behaviour ("agentd escalates them…").
    That path has now been measured on the reference machine, and it
    does not hold — which is why the `read` filter stays, and why #216
    stays open. What was run, on 2026-08-04, against the live daemons:
    - `RequestCall(app.lisaos.Surfer, navigate, …)` with
      `provenance: ["user"]` parks as **`confirm-modal`**,
      `effective_tier: destructive`, `escalated: true`. The tier
      machinery is real, and a `web` tag in the chain rides along.
    - `dev.lisaos.Consent1` has **no owner**. It is D-Bus-activatable,
      and agentd asks with `GetNameOwner`, which deliberately does not
      activate — so `consent_role()` answers `Absent`.
    - `Absent` is the headless fallback (#135): the requester may
      answer its own call. The probe's *own connection* then called
      `Confirm(call_id, true)` and agentd **dispatched it** — it came
      back `failed` only because the Surfer socket was dead (#219), an
      MCP transport error, not a refusal.
    So the only thing between a model and `click`/`fill` today would be
    `bus-tools` choosing not to call `Confirm` — code in the same
    process the model drives. CLAUDE.md rule 6a: if it is reachable
    from inside, it is not a guardrail. Exposing the write tier needs a
    consent surface that is *running* and *independent*, proven on a
    seated session, first.
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
- **There is no `download` tool on the Agent Bus, and that is a
  decision** (2026-08-05). A download tool is not a browser feature the
  model can use; it is *arbitrary bytes, from an address the model chose,
  written to a path on the person's disk* — a primitive nothing else in
  the system hands a model, and one the guard's path rules
  (`cli/lisa/src/guard.rs`) were not shaped for. The tier table would
  call it `write`, and `write` means "changes something the person can
  see and undo"; a file arriving in `~/Downloads` is neither. Adding it
  is a system-level question — *where may a model write, and how does the
  person see it afterwards* — and therefore an ADR, not a browser commit.
  What is implemented instead is the narrower rule, because **`navigate`
  and `click` are already enough**: an http address that answers
  `Content-Disposition: attachment` writes a file without any tool called
  `download`. A view is stamped whenever an agent-driven action touches
  it (`stampAgent`), and `agentDriven` in `lib/downloads.js` cancels any
  download that starts inside a five-second window after that stamp.
  Verified on the device 2026-08-05: `navigate` at
  `http://127.0.0.1:8899/hello2.bin` logged
  `refused an agent-driven download` and wrote nothing, while the same
  URL opened as a launch argument — a person asking — downloaded
  normally. The refusal is deterministic code the model cannot reach
  (CLAUDE.md 6a), not a tool left off a list.
- **Downloads: what is NOT there.** No pause/resume (WebKit offers no
  resume on `WebKitDownload`), no per-download "save as…" chooser — the
  destination is XDG Downloads and the only question asked is the
  conflict one — and no open-with picker beyond the desktop default. A
  `running` download is written to disk as `interrupted`, so a transfer
  killed with the process does not come back as a progress bar for
  something that is not happening.
- **Still not built**: passwords and form autofill (ADR-0037 leaves the
  credential story explicitly undecided, and a rushed one is worse than
  none), per-site zoom, reader mode, per-download destination choice, and
  a profile switcher — `lib/profiles.js` and `lib/store.js` both key off
  a profile name, but the window only ever uses `personal`.
- **History records `file:` URLs.** A person browsing their own machine
  is their business (ADR-0029), and the row can be deleted like any
  other — but it does mean local paths appear in `history.json`.
- **Find, zoom and print are device-verified as WebKit calls, not as
  keystrokes.** GNOME 50 refuses remote screenshots and nothing can press
  Ctrl+F over SSH, so a probe on the reference machine drove
  `FindController` with the app's own `findOptions()` (options `17`, two
  matches for a word on the page, `failed-to-find-text` for one that is
  not) and `set_zoom_level` with the app's own step list. That the
  *bar* appears when Ctrl+F is pressed is structure, not function, and
  the first seated session judges it.
- `tests/` cover the pure modules; the window itself is verified by
  eyes on hardware. **A unit test cannot prove world isolation** —
  `tests/world.test.js` pins the `world_name` argument, and the
  isolation itself was proved on the reference device with a page that
  redefines `JSON.stringify` and `document.querySelector`: before the
  fix `read_page` reported the page's forged title and an approved
  `fill` on `#q` landed in `#pw`; after it, the real title and the real
  field. Any future change to how agent scripts are evaluated needs
  that run again, not just a green suite.
