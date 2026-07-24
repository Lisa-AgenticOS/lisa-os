# ADR-0015: a persistent Assistant chat window — the surface that makes the model usable

- **Status:** accepted
- **Date:** 2026-07-24

## Context

The directive (project owner, 2026-07-24): *"we don't have a GUI for `lisa
ask`, no chat window — how can we test the LLM without those? Plan and build as
a whole."*

The substrate works: `lisa-inferenced` streams real tokens, remote providers +
OAuth (Claude/ChatGPT) landed, every turn is ledgered. But the only
conversational surface is the transient Super+Shift+Space overlay
(`shell/overlay-extension`, §5.7.1) — one-shot, no history, no model choice.
There is nowhere to *live with* the model: no multi-turn chat, no way to pick
between the tiny local model and the cloud models the OAuth work just enabled.
So the whole stack is effectively untestable as a product, and the new
providers can't be exercised.

PLAN §5.8 deliberately ships **no chatbot** ("apps are proof of the SDK, not a
suite for its own sake") — AI is meant to live *inside* apps, with the overlay
for quick asks. That stance is right for the app suite, but it left a real hole:
no reference surface for inference + providers + ledger, and no way to test them.

## Decision

Ship **Lisa Assistant** (`shell/assistant/`), a persistent multi-turn chat
window, as a deliberate, scoped exception to §5.8. It is justified not as a
"chatbot suite" but as the **reference frontend and test harness** for the
whole inference path; it complements, not replaces, the overlay.

- **GJS + GTK4/Adwaita**, modelled on `shell/ledger-app` — not Flutter.
  Rationale (project owner's call): it's already on the GNOME desktop (no
  Flatpak/Wayland/embedder work, which the Flutter lane still lacks), and —
  being interpreted — it iterates on the real hardware by copying files, with
  no image rebuild. The Flutter lane (`lisa_ui`/`lisa_flutter`, ADR-0004/0014)
  remains the path for the *app suite*; this is not that.
- **A second frontend of the `org.lisa.Overlay1` backend**, honoring the
  overlay's "one headless backend, many thin frontends" design. The window
  sends `Ask(prompt, {lane:"chat", model_hint, history_json})` and renders the
  streamed `Token`/`Finished` signals — the same contract the overlay uses.
- **The backend gains a chat lane** (`_runChat`): multi-turn, no Agent-Bus
  action pass, talking to `lisa-inferenced`'s OpenAI-compat endpoint so the
  model's chat template applies and `remote:<provider>:<model>` routes through
  the egress broker. Existing overlay behavior is untouched (gated on
  `lane:"chat"`).
- **Local + cloud from the start.** Local models from `GET /v1/models`; cloud
  from `org.lisa.Remote1` (providers signed in or holding a key → `ListModels`).
  This requires enabling the per-user companion's remote routing
  (`cfg.remote.enabled` + `LISA_REMOTED_SOCKET` → the user broker socket).
- **Nothing hidden.** Every turn is ledgered by the daemon (`inference.*`
  local, `remote.*` cloud); the window shows an egress marker (ADR-0008 amber
  `#E66100`) on turns that leave the machine. The app renders; the daemons
  enforce.

## Consequences

- The transient overlay and the persistent window share one backend and one
  streaming contract — no duplicated inference plumbing.
- The tiny pinned local model (qwen3-0.6b) makes local replies weak; the cloud
  path (the OAuth work) is where the assistant is actually useful — hence
  local+cloud, not local-only.
- Streaming *through* the broker is still non-streaming today (re-chunked for
  feel, ADR-0010); true remote streaming is a follow-up.
- Packaging is plain GJS files + a `.desktop` (like `ledger-app`), no new
  toolchain. The one image/unit change is the companion's remote routing.
- This is the first step of "build as a whole": the app suite (Notes, Recorder,
  the Files/Mail/Photos/Terminal patches, §5.8) rides the same daemons next.
