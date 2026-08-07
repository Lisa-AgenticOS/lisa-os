# ADR-0061 — Lisa Coder grows from forge-harness; other harnesses are quarries

- **Status:** accepted
- **Date:** 2026-08-07
- **Owner decision:** "Good we will only steal good parts from harnesses."
- **Relates to:** ADR-0025 (one agent loop), ADR-0029/0030 (guardrails are
  deterministic code outside the model), ADR-0047 (forge harness), PLAN
  §5.12.1 (bring-your-own agent backend).
- **Claims:**
  - `path:libs/forge-harness` — the loop Lisa Coder grows from
  - `path:cli/lisa/src/guard.rs` — the deny catalogue no adopted harness may bypass

## Context

The coding agent for Lisa ("Lisa Coder") could be adopted, forked, or
grown. The strongest external candidate was evaluated on 2026-08-07:
jcode (github.com/1jehuang/jcode, MIT, Rust, 16k stars) — 690k lines,
84 crates, 7,900 tests, streaming loop with compaction, async durable
memory, swarm fan-out, and an OpenAI-compat provider that can point at
`lisa-inferenced` today.

The evaluation killed adoption and forking on Lisa's own rules, not on
quality:

- **Inverted safety model.** jcode's own design doc: *"there is no
  'always denied' — if the user explicitly approves something, the
  agent can do it."* Lisa's guard is built on the opposite invariant
  (unoverridable `Deny`, ADR-0030), with consent a model host cannot
  self-approve (ADR-0033) — properties that exist in ~14k audited,
  corpus-tested lines in-tree.
- **Egress woven through the core.** Default-on telemetry with no
  compile-out, update checks, a sponsored-discovery service, and
  direct-to-search websearch — four independent violations of rule 5,
  threaded through its two largest crates rather than isolated.
- **Bus factor 1 at 30–116 commits/day.** 99.8% of 6,810 commits are
  one author; 982 locked dependencies. A fork diverges daily from a
  single-maintainer codebase — the harness-shaped version of the
  mistake rule 11 exists to prevent.

## Decision

1. **Lisa Coder is forge-harness, grown.** The loop, the jail, the
   guard integration and the ledger mandate stay ours; capability gaps
   are closed by construction, in-tree.
2. **Other harnesses are quarries.** Proven *ideas* migrate — as our
   code, with tests watched red first — never vendored subsystems. The
   first four steals (from jcode): async memory surfacing through
   contextd, the reflection-prompt on guard soft-denies, swarm ancestry
   bookkeeping, and SDK-dogfooded-by-a-shipping-client discipline.
3. **The BYO seam is the polish-gap mitigation.** PLAN §5.12.1's
   one-sentence promise — any agent CLI as a forge backend inside the
   same tool jail — becomes real work: the jail wraps the foreign
   agent's process, the guard wraps its shell, egress stays
   broker-only. Frontier agents stay usable on Lisa without becoming
   load-bearing.

## Consequences

- No third-party harness enters the dependency graph of install,
  update, recovery, or the OS image (rule 7a unchanged).
- The steals and the seam are tracked work items, each its own commit;
  a steal that cannot be expressed under the deny-catalogue and
  broker-egress invariants is not taken.
- The evaluation itself (architecture, health, and fit evidence) lives
  in the task record; this ADR fixes only the decision and its reasons.

## Limits

- Lisa Coder will feel primitive next to mature harnesses for some
  time — session resume, compaction and a first-class TUI are each
  thousands of lines the quarries have already paid for. That cost is
  accepted; the BYO seam is the pressure valve while the native loop
  matures.
