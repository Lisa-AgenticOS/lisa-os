<script setup lang="ts">
// API reference — every signature read from source:
// daemons/inferenced/src/{api,dbus,openai}.rs, daemons/agentd/src/dbus.rs,
// daemons/contextd/src/dbus.rs, daemons/remoted/src/dbus.rs,
// shell/overlay-extension/lib/iface.js, apps/notes/app.lisaos.notes.json.
const repo = `https://github.com/${useRuntimeConfig().public.repo}`
useHead({ title: 'API reference — Lisa OS developers' })
</script>

<template>
  <div>
    <header class="pagehead">
      <span class="eyebrow">API reference</span>
      <h1>The surfaces you build against.</h1>
      <p class="lede">Three layers: an OpenAI-compatible HTTP endpoint on loopback (zero Lisa-specific dependencies), session D-Bus interfaces under <code>dev.lisaos.*</code>, and MCP tool manifests for exposing app actions to the Agent Bus. Everything below is read from the source on <code>main</code>.</p>
      <nav class="jump">
        <a href="#http">HTTP (OpenAI-compat)</a>
        <a href="#inference1">Inference1</a>
        <a href="#agent1">Agent1</a>
        <a href="#context1">Context1</a>
        <a href="#remote1">Remote1</a>
        <a href="#overlay1">Overlay1</a>
        <a href="#mcp">MCP tools</a>
      </nav>
    </header>

    <section id="http" class="sec anchor">
      <h2>OpenAI-compatible HTTP — <code>127.0.0.1:7777</code></h2>
      <p>Served by <code>lisa-inferenced</code> (<a :href="`${repo}/blob/main/daemons/inferenced/src/api.rs`">source</a>). Any OpenAI client works unmodified — point it at the base URL, any API key. The system instance owns port 7777; the per-user companion (which can route to cloud providers) owns 7778. Every generate/embed is gated by the Ledger: the entry precedes the action, and if the append fails the request is refused with 503.</p>

      <h3>GET /health</h3>
      <pre><code>curl 127.0.0.1:7777/health
<span class="c">→ {"status":"ok","engine":"llama","version":"…"}   # engine: "stub" or "llama"</span></code></pre>

      <h3>GET /v1/models</h3>
      <pre><code>curl 127.0.0.1:7777/v1/models
<span class="c">→ {"object":"list","data":[{"id":"…","object":"model","created":…,"owned_by":"lisa"}]}</span></code></pre>

      <h3>POST /v1/chat/completions</h3>
      <p>Request fields (from the wire types): <code>model</code> (optional — defaults to the resident system model), <code>messages</code> (<code>[{role, content}]</code>), <code>stream</code> (bool), <code>response_format</code>, <code>lisa_priority</code> (<code>"interactive"</code> | <code>"background"</code> — background requests are preempted by interactive ones), <code>max_tokens</code>.</p>
      <pre><code>curl 127.0.0.1:7777/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"messages":[{"role":"user","content":"write a haiku about entropy"}]}'
<span class="c">→ {"id":"chatcmpl-lisa-…","object":"chat.completion","created":…,"model":"…",
   "choices":[{"index":0,"message":{"role":"assistant","content":"…"},"finish_reason":"stop"}],
   "usage":{"prompt_tokens":0,"completion_tokens":…,"total_tokens":…}}</span></code></pre>
      <p><strong>Streaming</strong> (<code>"stream": true</code>): Server-Sent Events, OpenAI chunk convention — a role preamble chunk, then <code>delta.content</code> token chunks, a <code>finish_reason: "stop"</code> chunk, then <code>data: [DONE]</code>. Errors mid-stream arrive as a <code>{"error":{"message":…}}</code> data event.</p>
      <p><strong>Guided generation</strong> — the flagship feature. Hand it a JSON Schema and the output is grammar-constrained (JSON Schema → GBNF, enforced by the sampler), so it always parses:</p>
      <pre><code><span class="k">from</span> openai <span class="k">import</span> OpenAI
client = OpenAI(base_url=<span class="k">"http://127.0.0.1:7777/v1"</span>, api_key=<span class="k">"local"</span>)

r = client.chat.completions.create(
    model=<span class="k">"lisa"</span>,
    messages=[{<span class="k">"role"</span>: <span class="k">"user"</span>, <span class="k">"content"</span>: <span class="k">"Extract the recipe.\n\n"</span> + text}],
    response_format={<span class="k">"type"</span>: <span class="k">"json_schema"</span>,
                     <span class="k">"json_schema"</span>: {<span class="k">"name"</span>: <span class="k">"recipe"</span>, <span class="k">"schema"</span>: SCHEMA}})
<span class="c"># always valid JSON for SCHEMA</span></code></pre>
      <p>An unsupported schema returns 400 with <code>invalid_request_error</code>. Non-streaming guided requests get one server-side re-sample if the output isn't valid JSON — structured output is the contract.</p>

      <h3>POST /v1/embeddings</h3>
      <pre><code>curl 127.0.0.1:7777/v1/embeddings \
  -H 'Content-Type: application/json' \
  -d '{"input": ["first text", "second text"]}'
<span class="c">→ {"object":"list","model":"…",
   "data":[{"object":"embedding","index":0,"embedding":[…]}, …],
   "usage":{"prompt_tokens":0,"total_tokens":0}}</span></code></pre>
      <p><code>input</code> is a string or an array of strings (anything else → 400).</p>
    </section>

    <section class="sec anchor">
      <h2>D-Bus interfaces</h2>
      <p>Session-bus services under <code>dev.lisaos.*</code>. Rich results are JSON strings — one serialization, so <code>busctl</code> and scripts read them directly. All are tested over zbus peer-to-peer connections and registered on the session bus on real systems.</p>
      <div class="note amber">Naming note (ADR-0016): the source on <code>main</code> uses <code>dev.lisaos.*</code> / <code>app.lisaos.*</code>; release v20260724.25 still carries the older <code>org.lisa.*</code> names. The rename ships with the next release.</div>

      <h3 id="inference1">dev.lisaos.Inference1 — inference sessions</h3>
      <p>Object path <code>/dev/lisaos/Inference1</code> (<a :href="`${repo}/blob/main/daemons/inferenced/src/dbus.rs`">source</a>). The fd-stream contract: <code>OpenSession</code> returns a session object path and the <em>read end of a pipe</em>; tokens stream over that fd as raw UTF-8, and the daemon closes its write end when generation completes — <strong>EOF is end-of-message</strong>.</p>
      <pre><code>Ping() → s                                <span class="c"># "lisa-inferenced &lt;version&gt;"</span>
OpenSession(a{sv} options) → (o path, h fd)
    <span class="c"># options: "model_hint" (s) selects a resident model</span>

<span class="c"># on the returned session object (dev.lisaos.Inference1.Session):</span>
Generate(s prompt, a{sv} params)
    <span class="c"># params: "schema" (s, JSON Schema → grammar-constrained output),</span>
    <span class="c">#         "max_tokens" (u), "priority" ("interactive"|"background")</span>
    <span class="c"># tokens stream over the session fd; fd closes at end-of-message</span>
Embed(as texts) → aad                     <span class="c"># array of array of double</span>
Cancel()                                  <span class="c"># abort in-flight generation → early EOF</span>
Close()                                   <span class="c"># release the session object path</span></code></pre>

      <h3 id="agent1">dev.lisaos.Agent1 — the Agent Bus</h3>
      <p>Object path <code>/dev/lisaos/Agent1</code>, served by <code>lisa-agentd</code> (<a :href="`${repo}/blob/main/daemons/agentd/src/dbus.rs`">source</a>). Read-tier calls with a fully trusted chain execute immediately; everything else parks and emits <code>ConfirmationRequested</code>. Every path is ledgered before anything happens.</p>
      <pre><code>Ping() → s
ListTools() → (s tools_json)              <span class="c"># [{app_id, name, tier, description, undoable}]</span>
Discover(s query) → (s tools_json)        <span class="c"># rank tools against a natural-language query</span>
RequestCall(s app_id, s tool, s args_json, a{sv} options)
    → (t call_id, s disposition, s detail_json)
    <span class="c"># options: "actor" (s), "provenance" (as — the trigger chain;</span>
    <span class="c">#          omitted/empty = unknown = escalates one tier, rule 6)</span>
    <span class="c"># disposition: "executed" | "failed" | "confirm-chip" |</span>
    <span class="c">#              "confirm-modal" | "denied"</span>
Confirm(t call_id, b approve) → (s status, s detail_json)
    <span class="c"># status: "executed" | "failed" | "denied"</span>
Undo() → (s report_json)                  <span class="c"># revert via the journaled compensation</span>
signal ConfirmationRequested(t call_id, s spec_json)
    <span class="c"># spec_json carries the typed-diff material (tool, args, tiers, chain)</span></code></pre>

      <h3 id="context1">dev.lisaos.Context1 — the context fabric</h3>
      <p>Object path <code>/dev/lisaos/Context1</code>, served by <code>lisa-contextd</code> (<a :href="`${repo}/blob/main/daemons/contextd/src/dbus.rs`">source</a>). Every search appends a <code>context.search[.hybrid|.scoped]</code> ledger entry <em>before</em> the store is queried — if the append fails, the retrieval does not happen. Per-app memory is namespace-isolated: every method takes the app id and no call can cross it.</p>
      <pre><code>Ping() → s
Search(s query, a{sv} options) → (s hits_json)
    <span class="c"># options: "limit" (u, default 3), "hybrid" (b, BM25×cosine blend),</span>
    <span class="c">#          "scopes" (as — present ⇒ ACL-scoped retrieval,</span>
    <span class="c">#          deny-by-default on empty/unknown scopes)</span>
    <span class="c"># hits_json: [{source, provenance, snippet, score}]</span>
MemoryGet(s app, s key) → s               <span class="c"># missing key → error</span>
MemorySet(s app, s key, s value)
MemoryList(s app) → s                     <span class="c"># JSON object, key → value</span>
MemoryWipe(s app)                         <span class="c"># zero residual rows</span></code></pre>

      <h3 id="remote1">dev.lisaos.Remote1 — the egress broker</h3>
      <p>Interface <code>dev.lisaos.Remote1</code> at <code>/dev/lisaos/Remote1</code>; note the well-known bus name is <code>dev.lisaos.Remoted</code> (<a :href="`${repo}/blob/main/daemons/remoted/src/dbus.rs`">source</a>). The Settings app's management plane: providers, credentials (write-only — no method ever returns key material), per-scope offload consent, and "Sign in with Claude / ChatGPT" OAuth.</p>
      <pre><code>Ping() → s
State() → s                               <span class="c"># providers + credential presence + consent, one JSON doc</span>
AddProvider(s id, s display_name, s base_url)   <span class="c"># user-supplied OpenAI-compat endpoint</span>
RemoveProvider(s id)
SetKey(s id, s key)                       <span class="c"># write-only credential store</span>
ClearKey(s id)
SetConsent(s scope, b allowed)            <span class="c"># scopes: prompt|files|mail|calendar|screen|memory,</span>
                                          <span class="c"># default: nothing leaves</span>
BeginLogin(s provider_id) → s             <span class="c"># authorize URL ("anthropic" or "openai");</span>
                                          <span class="c"># completion arrives via LoginCompleted</span>
Logout(s provider_id)                     <span class="c"># forget a stored OAuth session (idempotent)</span>
ListModels(s provider) → s                <span class="c"># the provider's live /models, JSON array of ids</span>
signal LoginCompleted(s provider_id, b ok, s detail)
    <span class="c"># no token material is ever carried</span></code></pre>

      <h3 id="overlay1">dev.lisaos.Overlay1 — the assistant overlay backend</h3>
      <p>Bus name <code>dev.lisaos.Overlay1</code> at <code>/dev/lisaos/Overlay1</code> (<a :href="`${repo}/blob/main/shell/overlay-extension/lib/iface.js`">source</a>) — the headless backend shared by every thin frontend (the GNOME Shell extension, the Assistant chat window). <code>Ask()</code> returns a query id immediately; tokens arrive as <code>Token</code> signals and the turn ends with <code>Finished</code>.</p>
      <pre><code>Ask(s prompt, a{sv} options) → (t query_id)
Cancel(t query_id)                        <span class="c"># on a query awaiting consent, answers "deny"</span>
Respond(t query_id, b approve)            <span class="c"># answer a ConfirmationNeeded</span>
GetStatus() → a{sv}

signal Started(t query_id, s meta_json)
signal Token(t query_id, s text)
signal ConfirmationNeeded(t query_id, s spec_json)
signal Finished(t query_id, s status, s detail)</code></pre>
      <p>Options (<code>a{sv}</code>): the per-invocation context affordances as booleans — <code>"my_stuff"</code> (Context Fabric retrieval), <code>"window"</code> (screen capture → VLM; lands M6, currently reported unavailable), <code>"selection"</code> (app resource / AT-SPI; reported unavailable) — plus <code>"model_hint"</code> (s).</p>
      <p><strong>The chat lane</strong> (used by the persistent Assistant window) adds three options: <code>"lane"</code> = <code>"chat"</code> selects the multi-turn chat lane (no Agent pass; talks to the OpenAI-compat endpoint so the chat template applies and <code>remote:&lt;provider&gt;:&lt;model&gt;</code> routes through the broker), <code>"history_json"</code> (prior <code>[{role, content}]</code> turns), and <code>"model_hint"</code> (a local model id or <code>remote:&lt;provider&gt;:&lt;model&gt;</code>). Tokens and Finished are emitted exactly as for the inference lane.</p>
      <p>A companion frontend-owned interface, <code>dev.lisaos.Overlay1.UI</code> at <code>/dev/lisaos/Overlay1/UI</code>, offers <code>Summon(s prompt, a{sv} options)</code>, <code>Hide()</code>, and <code>GetVisible() → b</code> — the launcher's Spotlight-style "Ask Lisa" handoff.</p>
    </section>

    <section id="mcp" class="sec anchor">
      <h2>MCP tools — the app manifest</h2>
      <p>Apps expose actions to the Agent Bus by declaring typed tools in a manifest (PLAN §5.4, Appendix B). The Notes app ships the worked example, <a :href="`${repo}/blob/main/apps/notes/app.lisaos.notes.json`">apps/notes/app.lisaos.notes.json</a> (abridged):</p>
      <pre><code>{
  <span class="k">"lisa_manifest"</span>: 1,
  <span class="k">"app_id"</span>: <span class="k">"app.lisaos.notes"</span>,
  <span class="k">"mcp"</span>: { <span class="k">"transport"</span>: <span class="k">"unix"</span>, <span class="k">"activatable"</span>: false },
  <span class="k">"tools"</span>: [
    {
      <span class="k">"name"</span>: <span class="k">"create_note"</span>,
      <span class="k">"tier"</span>: <span class="k">"write"</span>,
      <span class="k">"description"</span>: <span class="k">"Create a note with a title and optional body"</span>,
      <span class="k">"input_schema"</span>: {
        <span class="k">"type"</span>: <span class="k">"object"</span>, <span class="k">"required"</span>: [<span class="k">"title"</span>],
        <span class="k">"additionalProperties"</span>: false,
        <span class="k">"properties"</span>: {
          <span class="k">"title"</span>: { <span class="k">"type"</span>: <span class="k">"string"</span>, <span class="k">"maxLength"</span>: 120 },
          <span class="k">"body"</span>:  { <span class="k">"type"</span>: <span class="k">"string"</span>, <span class="k">"maxLength"</span>: 4000 }
        }
      },
      <span class="k">"undo"</span>: { <span class="k">"tool"</span>: <span class="k">"delete_note"</span>, <span class="k">"map"</span>: { <span class="k">"id"</span>: <span class="k">"$result.id"</span> } }
    },
    { <span class="k">"name"</span>: <span class="k">"list_notes"</span>,   <span class="k">"tier"</span>: <span class="k">"read"</span>,  <span class="c">…</span> },
    { <span class="k">"name"</span>: <span class="k">"search_notes"</span>, <span class="k">"tier"</span>: <span class="k">"read"</span>,  <span class="c">…</span> },
    { <span class="k">"name"</span>: <span class="k">"delete_note"</span>,  <span class="k">"tier"</span>: <span class="k">"write"</span>,
      <span class="k">"undo"</span>: { <span class="k">"tool"</span>: <span class="k">"restore_note"</span>, <span class="k">"map"</span>: { <span class="k">"id"</span>: <span class="k">"$input.id"</span> } } },
    { <span class="k">"name"</span>: <span class="k">"restore_note"</span>, <span class="k">"tier"</span>: <span class="k">"write"</span>, <span class="c">…</span> }
  ]
}</code></pre>
      <ul>
        <li><strong><code>tier</code></strong> sets the confirmation policy, enforced at the bus: <code>read</code> → silent (ledgered), <code>write</code> → inline confirmation chip, <code>destructive</code> → explicit modal with a typed diff.</li>
        <li><strong><code>input_schema</code></strong> is a JSON Schema; the bus validates arguments before dispatch.</li>
        <li><strong><code>undo</code></strong> declares the compensating call, with <code>$input.*</code> / <code>$result.*</code> mappings journaled at execution — this is what powers <code>lisa undo</code> and <code>Agent1.Undo()</code>.</li>
      </ul>
      <p>Try it from the terminal:</p>
      <pre><code>lisa tools                                              <span class="c"># list registered tools</span>
lisa call app.lisaos.notes create_note '{"title":"milk"}'
lisa undo                                               <span class="c"># reverts via the declared compensation</span></code></pre>
    </section>
  </div>
</template>
