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

import {IFACE_XML} from '../../lisa.sdk/bus/interfaces.js';
import {BUS} from '../../lisa.sdk/bus/addresses.js';

// The three SYSTEM interfaces this surface speaks come from the sdk's
// one copy (ADR-0060, introspected from the running daemons) — the XML
// blocks that lived here are re-exports now, so the backend exports and
// every frontend consumes the same node the daemons actually serve.
export const OVERLAY_IFACE_XML = IFACE_XML.Overlay1;
export const OVERLAY_BUS_NAME = BUS.Overlay1.name;
export const OVERLAY_OBJECT_PATH = BUS.Overlay1.path;
export const VOICE_IFACE_XML = IFACE_XML.Voice1;
export const VOICE_BUS_NAME = BUS.Voice1.name;
export const VOICE_OBJECT_PATH = BUS.Voice1.path;
export const HARNESS_IFACE_XML = IFACE_XML.Harness1;
export const HARNESS_BUS_NAME = BUS.Harness1.name;
export const HARNESS_OBJECT_PATH = BUS.Harness1.path;



// dev.lisaos.Voice1 — push-to-talk capture (PLAN §5.7.5).
//
// This lives on the BACKEND, not the extension, for one hard reason:
// recording and transcription are a subprocess and a ~1s whisper run,
// and anything that blocks in the extension blocks the compositor —
// every window on the machine stops redrawing while Lisa listens. The
// extension therefore only holds the key and paints the indicator.
//
// Start/Stop rather than a duration, because push-to-talk is defined by
// the key: you speak for as long as you hold it. `lisa listen` takes a
// ceiling instead, which is the right shape for a terminal and the
// wrong one for a key.
//
// Transcribed carries the text rather than the audio path: the audio is
// deleted as soon as it has been transcribed, and a surface that never
// receives a path cannot accidentally keep one.


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
/// dev.lisaos.Harness1 — the one agent loop, as a service (ADR-0025).
///
/// The Assistant window drives THIS rather than Overlay1's chat lane:
/// the chat lane has no tools by construction, so an assistant asked
/// about the open page could only say it had no way to look.
///
/// Deliberately shaped like Ask/Token/Finished, so adopting it was a
/// change of destination rather than a rewrite of the rendering.


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
