# libs/bus-tools — the Agent Bus tool family

Spec: `docs/PLAN.md` §5.4, §5.10. Design: ADR-0025 (one agent loop),
ADR-0036 (triggers and trust), ADR-0030 (the guardrail boundary).

## What it does

It is the adapter between an agent loop and the Agent Bus. Every surface
that runs the loop — `lisa-harnessd` (the Assistant), `lisa assist`, and
the end-to-end tests — builds its tool catalog through this crate, so
all of them offer the same tools under the same rules.

Four jobs, and only the last two are about safety:

1. **Catalog.** `ListTools` JSON → `Vec<BusTool>`, filtered by
   [`Offer`]. A row with no tier, or a tier we do not recognise, is
   dropped rather than defaulted to `read`.
2. **Naming.** Bus ids are `app.lisaos.notes::create_note`, which is not
   a legal OpenAI tool name. `wire_name` flattens them reversibly.
3. **The chain.** Every `RequestCall` this crate makes carries a
   provenance chain: what woke the run up, followed by every untrusted
   class the run has since consumed.
4. **The taint.** A shared, one-way `Taint` set. Nothing removes a tag —
   the model has read the content and nothing un-reads it.

**This crate is not a security boundary and must not be read as one.**
It decides what is *useful* to offer and what the chain *says*; agentd
decides what may happen, in a different process, from peer credentials
the model cannot reach (CLAUDE.md rule 6a). `offerable_tools` and
`write_tier_allowed` are product decisions with safety-shaped names.

## How it works

The smallest real usage is the loop's own:

```rust
let Some(tools) = bus_tools::AgentBusTools::discover()? else { return Ok(()) };
let providers: Vec<&dyn forge_harness::ToolProvider> = vec![&tools];
forge_harness::forge_agent_with_tools(prompt, cwd, &mut backend, &config, &providers, &mut |_| {});
```

`discover()` assumes a person is typing. A run woken by anything else
uses `discover_with_trigger(class)`, where `class` is the caller's
**resolved** trigger from `lisa-harnessd`'s `caller.rs` — never a class
the message asked for.

The taint half, which is the part worth reading:

```rust
// in `ToolProvider::execute`, after the bus answers
if disposition == "executed" && let Some(tag) = untrusted_result_provenance(&detail) {
    self.taint.add(&tag);
}
// …and every later call leaves with
let mut chain: Vec<&str> = vec![self.trigger];
chain.extend(self.taint.tags().iter().map(String::as_str));
```

So a run that reads a message and then tries to archive one arrives at
agentd as `["user", "mail"]`. `tier::resolve` raises the call one tier,
`Confirmation::for_tier` turns a chip into a modal, and `bus::grant_for`
stops calling it a `Trigger::Prompt` run — which is what keeps it off
the person's own filesystem reach (#252).

Sharing the set across families is deliberate: `Taint` is a property of
the *conversation*, not of the provider that acquired it, because the
model reads everything in one context. `harnessd`'s memory digest adds
to the same set when it serves back a note that was written from
untrusted content (#157).

## How to extend it

- **A new tool tier** goes in `Offer` and `offerable_tools`. Destructive
  stays out of every slice; that is a product decision about
  confirmation fatigue (ADR-0030), not a claim the tier machinery is
  untrusted.
- **A new provenance class** needs *nothing here* — that is the point of
  #302's fix, and it is the only design in this file that is meant to
  need no maintenance. `untrusted_result_provenance` taints on anything
  that is not `user`. Add the variant to `agentd`'s `Provenance` so the
  enforcement side names it, add a corpus entry, and
  `tests/injection-suite/tests/loop_taint.rs` will refuse to compile
  until the new tag has actually been driven through a loop.
- **A new transport** implements `BusTransport`. The trait exists so the
  loop can be driven into a real `AgentBus` with no D-Bus daemon and no
  desktop; a provider that can only be exercised on a live session bus
  is a provider nobody exercises.

## Limits

- **`web` was the only provenance that counted, until now (#302).**
  This is worth stating plainly because two component READMEs described
  the fixed behaviour as if it were the shipped behaviour for over a
  month. `apps/mail/README.md` said the `mail` tag "is what makes 'read
  my mail, then do something privileged' ask first", and
  `apps/preview/README.md` said a Write-tier call after reading a
  document "escalates to a confirmation". **Both claims were false on
  the agent-loop path.** The apps tagged their results correctly; this
  crate compared the tag to the literal string `web` and discarded
  `mail`, `file`, `screen` and everything else, so the chain reaching
  agentd stayed `["user"]`. Only Surfer's promise was ever kept. The
  claims are true as of this commit; they were documentation of intent
  before it, which CLAUDE.md rule 10 calls this repo's most repeated
  defect.
- **An app can still decline to taint, by tagging its result `user`.**
  `user` is the one spelling that costs nothing, so an app that puts it
  on a tool result buys silence. This is not new — `Taint::add` has
  refused `user` since #146 and the old code tainted on `web` alone, so
  the same app bought the same silence more cheaply — and it buys no
  *extra* trust either, because taint is one-way and nothing removes a
  tag that is already set. But it is a claim the loop takes at face
  value, and the loop is the wrong place to check claims: rule 6b says
  identity comes from the transport, and agentd's `verify_chain` is
  where a `user` claim is bound to peer credentials. Nothing binds the
  `user` claim on a *result*. No shipped app makes it.
- **The catalog is a snapshot.** An app that registers a tool mid-run is
  not picked up; the tool list is handed to the backend once.
- **The envelope's provenance is still discarded on the way in.**
  `libs/mcp-bus/src/client.rs`'s `extract_tool_result` unwraps
  `content[0].text` and drops the JSON-RPC envelope, so the whole scheme
  depends on every app tagging *inside* the payload as well as on the
  envelope. All three MCP apps do it, and each one has a comment
  explaining why — which is three copies of a workaround for one bug
  that has no issue of its own yet.
- **`Provenance` is not the single source of truth for tag spellings.**
  `daemons/contextd/src/acl.rs` knows `calendar` and `system`, which the
  enum does not; they parse to `Provenance::Other`, which is untrusted,
  so the outcome is right by accident of a fail-closed default rather
  than by anybody having reconciled the two lists.
