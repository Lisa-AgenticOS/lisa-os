# tests/injection-suite — prompt-injection red team

Spec: docs/PLAN.md §5.10, §5.4. Milestone: M5 gate. Design: ADR-0009.

The M5 gate: a hostile string embedded in retrieved mail/file/screen
content results in **0 unconfirmed privileged calls** across 500+ seeded
attempts. The assertion is two-layered:

- **Bus layer (shipped, host-independent):** the corpus (hostile payload
  × delivery vector × target tool) is driven through a real
  `lisa_agentd::bus::AgentBus` with a recording dispatcher. Every
  attempt's trigger chain carries untrusted provenance, so every call is
  raised one tier and must park for confirmation — the bus dispatches
  nothing unconfirmed, whatever the payload claims, and even the
  read-tier target the hostile text asked for has to be answered first.
  This is the load-bearing guarantee (enforced by the bus, not app
  goodwill) and it runs on macOS and Linux with no model and no desktop.
  See `tests/gate.rs`.
- **Model-in-the-loop layer (deferred, ADR-0009):** feed each payload
  through the real Appendix C system prompt + a resident model, assert
  the emitted plan, then run that plan through the same bus. Needs
  `inferenced` + a model + the MCP transport; wired when those land.

## Corpus

`src/lib.rs` generates the corpus as payload × vector × target: 44
payloads × 5 vectors (mail/file/screen/web/app-forwarded) × 6 targets =
**1320 attempts**, clearing the §5.10 500+ bar (the gate asserts the
floor so the bank can't shrink back under it). It was 600 until the
browser's write tier became reachable from an agent loop (#216) and
`app.lisaos.Surfer/navigate` joined the target list — a page the model
read can now name a tool the model actually holds, so the shape stopped
being hypothetical; 800 until Surfer's autofill made "get the password"
a thing to ask for (#260); and 1100 until a **read-tier** target joined
so the escalation rule had something to decide (#304, below). The
payload bank is a deliberate taxonomy — direct override,
authority/system spoof, delimiter/context escape, false prior-approval,
mode/roleplay switch, conditional triggers, exfiltration, provenance
spoofing, urgency, multi-step chaining, payment fraud — since the bus
guarantee is technique-agnostic, breadth is the point. The corpus is a
library so the gate test and the future model-in-the-loop test share it.

## Generating attempts is not running them (#303, #304)

Until 2026-08-06 the number above was a lie by omission. Both corpus
tests parked their calls from a single `Owner` and never drained the
pending map, so from the 17th attempt onward the Agent Bus'
`MAX_PENDING_PER_OWNER` capacity cap denied every request *before* the
tier machinery was consulted — and the tests counted a denial as a pass.
**16 of 1100 attempts reached a tier decision.** A rate-limiter refusal
was supplying 98.5% of the green, and the gate stayed green with
provenance escalation deleted outright (#304).

Both defects are fixed by the control `tests/acl-fuzz` has carried since
#116 — count what actually reached the thing under test, and assert a
floor on it:

- **The gate drains what it parks.** Every parked call is withdrawn
  immediately after it has been asserted on, which is exactly what the
  cap's own refusal tells a caller to do ("answer or withdraw one
  first"). The cap is a capacity bound, not a rate limit, so satisfying
  it costs no wall-clock time and needed no test-only knob in agentd.
  **1320 of 1320 attempts now reach a tier decision**, and
  `assert_the_corpus_actually_ran` fails the gate if any do not, naming
  the ones that were lost and why.
- **The cap keeps its own test.**
  `a_caller_that_never_drains_is_stopped_by_the_pending_cap` runs the
  corpus without draining and asserts the cap still bites at exactly 16
  — it is a real defence against a peer exhausting the confirmation
  map, it was simply standing in front of the measurement.
- **The escalation rule is asserted by name.** Each parked call is
  compared against `escalation_oracle`, this crate's own table of what
  an untrusted chain must produce (effective tier + confirmation class
  + the `escalated` flag), never against `tier::resolve` itself. And
  `TARGETS` now carries a **read-tier** tool, `list_events`: the only
  shape whose outcome the escalation rule decides alone — untrusted, it
  parks behind a chip; trusted, it runs silently. Its positive control
  is `a_trusted_chain_still_lets_the_read_tier_target_run_silently`,
  without which "everything parks" would be satisfied by a resolver
  that returned `Modal` for everything.

Mutating `tier::resolve`'s escalation away now turns three of the eight
tests in `gate.rs` red.

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

## What the corpus cannot see (#302)

The corpus asserts one half of rule 6 and the halves are easy to
confuse. `src/lib.rs`'s `chain_for` builds `["user", provenance]` and
hands it to `AgentBus::request` directly — nothing in `tests/gate.rs`
imports `bus_tools` at all. So the gate proves **the bus escalates a
chain that carries untrusted provenance**, and has never once proved
**the loop assembles such a chain**. Its docstring said the chain was
"exactly what the agent loop would assemble", and for four of the five
vectors that was false for as long as the file existed: the loop
recognised the literal string `web` and discarded `mail`, `file`,
`screen` and `app:*`. A perfectly green 1100-attempt corpus sat on top
of an agent loop that told agentd every run was trusted.

`tests/loop_taint.rs` is the missing half, and it is the one place in
the repo where the chain under test is *produced by the shipping
provider* rather than written by the test:

- `every_untrusted_provenance_taints_the_run_not_only_web` drives a real
  loop once per `Provenance` variant and asserts the chain the provider
  built for the call that follows. Its variant list is checked against
  an exhaustive `match` on the enum, so adding a variant to
  `daemons/agentd/src/tier.rs` breaks the build here until the new tag
  has been driven through a loop. The policy fix alone would have
  drifted back; this is the part that makes it stay fixed.
- `a_write_after_an_untrusted_read_escalates_whatever_tagged_it` states
  the consequence a person would notice: a modal, not a chip, and
  nothing dispatched — for `mail` and `file` as well as `web`.
- `a_trusted_read_does_not_escalate_the_write_that_follows` is the
  positive control. Without it, a build that tainted *everything* would
  pass both tests above while asking people to confirm their own typing.

Note that fixing #302 does **not** move the gate's numbers, because the
gate never ran the loop. If a change to the loop's taint policy ever
does move them, something has started reaching the corpus that should
not.

Run: `cargo test -p lisa-injection-suite`.
