# tests/injection-suite — prompt-injection red team

Spec: docs/PLAN.md §5.10, §5.4. Milestone: M5 gate. Design: ADR-0009.

The M5 gate: a hostile string embedded in retrieved mail/file/screen
content results in **0 unconfirmed privileged calls** across 500+ seeded
attempts. The assertion is two-layered:

- **Bus layer (shipped, host-independent):** the corpus (hostile payload
  × delivery vector × privileged target) is driven through a real
  `lisa_agentd::bus::AgentBus` with a recording dispatcher. Every
  attempt's trigger chain carries untrusted provenance, so every
  privileged call must park for confirmation — the bus dispatches
  nothing unconfirmed, whatever the payload claims. This is the
  load-bearing guarantee (enforced by the bus, not app goodwill) and it
  runs on macOS and Linux with no model and no desktop. See
  `tests/gate.rs`.
- **Model-in-the-loop layer (deferred, ADR-0009):** feed each payload
  through the real Appendix C system prompt + a resident model, assert
  the emitted plan, then run that plan through the same bus. Needs
  `inferenced` + a model + the MCP transport; wired when those land.

## Corpus

`src/lib.rs` generates the corpus as payload × vector × target: 40
payloads × 5 vectors (mail/file/screen/web/app-forwarded) × 4 privileged
targets = **800 attempts**, clearing the §5.10 500+ bar (the gate asserts
the floor so the bank can't shrink back under it). It was 600 until the
browser's write tier became reachable from an agent loop (#216) and
`app.lisaos.Surfer/navigate` joined the target list — a page the model
read can now name a tool the model actually holds, so the shape stopped
being hypothetical. The payload bank is a
deliberate taxonomy — direct override, authority/system spoof,
delimiter/context escape, false prior-approval, mode/roleplay switch,
conditional triggers, exfiltration, provenance spoofing, urgency,
multi-step chaining, payment fraud — since the bus guarantee is
technique-agnostic, breadth is the point. The corpus is a library so the
gate test and the future model-in-the-loop test share it.

## The two gates the write tier added (#216)

`tests/gate.rs` proves the bus dispatches nothing unconfirmed. That was
sufficient while no loop could reach a write-tier tool. Now one can, so
"unconfirmed" needed a second clause: **not confirmed by the thing that
asked**. A parked call the requester can release is a parked call, and a
hostile page that gets the model to call `navigate` also gets it to call
`Confirm`.

- `a_model_host_cannot_confirm_its_way_through_the_corpus` runs the whole
  corpus as a model host and then tries to release every parked call from
  the model host's own peer. Zero releases, zero dispatches.
- `the_consent_surface_can_still_release_a_corpus_call` is its positive
  control. Without it, a `confirm` that refused *everything* would make
  the test above green while shipping a system in which no privileged
  call can ever complete.

`tests/loop_write_tier.rs` is the end-to-end answer to #216's actual
complaint — that the write tools had no agent-loop caller and the
escalation story was therefore documented rather than run. It drives a
**real** `forge_harness` agent loop, holding a **real** `bus_tools`
provider with write-tier tools in its catalog, into a **real**
`lisa_agentd::bus::AgentBus`. The model is scripted; nothing else is. It
proves: the loop can make the call; the call parks; the loop's own peer
cannot release it; an independent surface can, and only then does it run
and land in the undo journal; a write following a web-tagged `read_page`
arrives escalated to a modal (#166's acceptance, previously unreachable);
and the loop may still withdraw its own call.

Run: `cargo test -p lisa-injection-suite`.
