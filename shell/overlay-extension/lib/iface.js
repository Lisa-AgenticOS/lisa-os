// dev.lisaos.Overlay1 — the headless overlay backend's D-Bus surface
// (docs/PLAN.md §5.7.1: "one headless overlay backend (session D-Bus
// service owning state/streams) with thin frontends").
//
// Shared by the backend (lisa-overlayd.js exports it) and every thin
// frontend (the GNOME Shell extension here; the wlr-layer-shell client
// for Track L consumes the same interface).
//
// Ask() returns a query id immediately; tokens arrive as Token signals
// and the turn ends with Finished. Options (a{sv}) carry the three
// per-invocation context affordances as booleans:
//   "my_stuff"  → Context Fabric retrieval (PLAN §5.3, via `lisa context search`)
//   "window"    → screen capture → VLM (PLAN §5.7.4 — lands M6, reported unavailable)
//   "selection" → app resource / AT-SPI (PLAN §5.7.3 layer 3 — reported unavailable)
// plus "model_hint" (s), forwarded to dev.lisaos.Inference1.
//
// The persistent chat window (a second frontend) adds three options:
//   "lane" (s)         → "chat" selects the multi-turn chat lane (no Agent
//                        pass; talks to lisa-inferenced's OpenAI-compat
//                        endpoint so the chat template applies and
//                        `remote:<provider>:<model>` routes through the broker)
//   "history_json" (s) → prior [{role,content}] turns, for multi-turn
//   "model_hint" (s)   → a local model id or `remote:<provider>:<model>`
// Tokens and Finished are emitted exactly as for the inference lane, so a
// frontend renders both the same way.
//
// Agent Bus lane (M5, ADR-0013): an actionable prompt routes to
// dev.lisaos.Agent1 instead of inference. The result (or the denial /
// failure reason) streams as Token signals and Finished carries the
// Agent1 disposition as its status — "executed" (detail = ledger ref
// or ''), "failed"/"denied" (detail = reason) — alongside the
// inference-path "ok"/"cancelled"/"error". A parked call raises
// ConfirmationNeeded(query_id, spec_json) — spec_json is Agent1's
// typed-diff material — and the frontend answers with
// Respond(query_id, approve). Confirmations parked by other clients
// (e.g. `lisa do` without a TTY answer) surface the same way: the
// backend subscribes to Agent1.ConfirmationRequested and gives each
// external call a query id. Cancel() on a query awaiting consent
// answers "deny".

export const OVERLAY_IFACE_XML = `
<node>
  <interface name="dev.lisaos.Overlay1">
    <method name="Ask">
      <arg type="s" name="prompt" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="t" name="query_id" direction="out"/>
    </method>
    <method name="Cancel">
      <arg type="t" name="query_id" direction="in"/>
    </method>
    <method name="Respond">
      <arg type="t" name="query_id" direction="in"/>
      <arg type="b" name="approve" direction="in"/>
    </method>
    <method name="GetStatus">
      <arg type="a{sv}" name="status" direction="out"/>
    </method>
    <signal name="Started">
      <arg type="t" name="query_id"/>
      <arg type="s" name="meta_json"/>
    </signal>
    <signal name="Token">
      <arg type="t" name="query_id"/>
      <arg type="s" name="text"/>
    </signal>
    <signal name="ConfirmationNeeded">
      <arg type="t" name="query_id"/>
      <arg type="s" name="spec_json"/>
    </signal>
    <signal name="Finished">
      <arg type="t" name="query_id"/>
      <arg type="s" name="status"/>
      <arg type="s" name="detail"/>
    </signal>
  </interface>
</node>`;

export const OVERLAY_BUS_NAME = 'dev.lisaos.Overlay1';
export const OVERLAY_OBJECT_PATH = '/dev/lisaos/Overlay1';

// dev.lisaos.Overlay1.UI — UI-control surface owned by a *frontend*
// (the GNOME Shell extension here; the wlr-layer-shell client can own
// the same name). Lets other shell surfaces summon the overlay with a
// prompt — the §5.7.2 launcher's "Ask Lisa" lane does the
// Spotlight-style handoff: overview closes, overlay opens with the
// query already submitted. The headless backend (dev.lisaos.Overlay1
// above) is deliberately not involved: it has no UI.
//
// Summon()'s options (a{sv}) accept the same chip booleans as Ask()
// ("my_stuff", "window", "selection") to preset the toggles; an empty
// prompt just shows the layer, exactly like Super+Shift+Space.
export const OVERLAY_UI_IFACE_XML = `
<node>
  <interface name="dev.lisaos.Overlay1.UI">
    <method name="Summon">
      <arg type="s" name="prompt" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
    </method>
    <method name="Hide"/>
    <method name="GetVisible">
      <arg type="b" name="visible" direction="out"/>
    </method>
  </interface>
</node>`;

export const OVERLAY_UI_BUS_NAME = 'dev.lisaos.Overlay1.UI';
export const OVERLAY_UI_OBJECT_PATH = '/dev/lisaos/Overlay1/UI';
