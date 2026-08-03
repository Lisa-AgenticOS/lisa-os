# ADR-0044: Retrieval receipts — contextd vouches for what it returned

- Status: **proposed** (design only, 2026-08-03 — the full design with
  sequencing is on #55; this ADR records the decision-shape so it
  survives sessions)
- Date: 2026-08-03
- Relates: ADR-0033 (identity from the transport), ADR-0029/0030
  (tiering + the boundary), #55 (the gap and the design), ADR-0043
  (`system` provenance rides the same mechanism)

## Context

agentd's tier machinery escalates on untrusted provenance — but the
provenance chain in `request_call` is the *caller's description* of
what context is in play. Post-#55 hardening binds the actor to peer
credentials and strips over-trusted claims, yet a caller that
retrieved a `mail` chunk through `dev.lisaos.Context1` can still
assert `["user"]`: the chain is data about a fact agentd never
witnessed, asserted by the one party with an incentive to shade it.

## Decision (shape)

contextd — the only party that knows what a retrieval returned —
issues a **receipt** with every search reply: a hash over a per-reply
nonce and the distinct provenances of the hits, remembered briefly in
an `Owner`-keyed log. Callers pass the receipt instead of
self-describing; agentd verifies it **with contextd directly** over a
private method gated by peer identity (ADR-0033: the bus connection is
the trust primitive — no signing keys for a same-machine proof). What
enters the tier computation is what contextd said, not what the caller
claimed. No receipt verifies = empty chain = fails closed.

Stated limit: receipts close the lying-about-what-was-retrieved hole,
not the lying-by-omission hole. A caller passing no receipt asserts
"no retrieved content", exactly as cheaply as today. Omission closes
only when the trusted composer lane (overlay/assistant, already
peer-verified Lisa programs) is the party attaching receipts — then an
untrusted app no longer speaks for its context at all.

## Why not the alternatives

Re-querying contextd is a TOCTOU against a moving index and would hand
agentd the query text the Ledger deliberately only hashes. Signed
provenance lists are key management for a proof the bus already
provides. Per-chunk id disclosure couples agentd to store internals
and leaks document identity; the receipt aggregates to exactly what
the tier machinery consumes.

## Acceptance (when implemented)

The injection-suite gains: an asserted `["user"]` alongside an
existing receipt for the conversation must not buy the trusted path; a
forged receipt fails closed; Ledger rows distinguish
`agent.provenance_receipt` from `agent.provenance_asserted` so the two
populations stay auditable apart.
