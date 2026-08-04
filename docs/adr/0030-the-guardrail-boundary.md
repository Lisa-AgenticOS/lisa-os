# ADR-0030: The guardrail boundary — probabilistic inside, logical outside

- **Status:** accepted — the principle has teeth rather than prose: #145 and
  #55 were both closed against it, and `lisa guard list|allow|forbid` is the
  owner's out-of-band relaxation, where no tool call can reach it.
- Date: 2026-07-26
- Amends: [ADR-0029](0029-hard-guardrails-for-agent-actions.md), which
  made `Verdict::Deny` absolute with no override at all
- Relates: PLAN §5.4 (Agent Bus), §5.10 + Appendix C (provenance,
  injection), `docs/VISION.md`, issues #55, #54, #53

## Context

ADR-0029 built a deterministic policy layer and gave it a `Deny` class
that nothing could override — no confirmation, no flag, no prompt. Three
adversarial review rounds later it works, and the reasoning behind that
absoluteness turned out to be half right.

The framing that clarified it came from a conference slide: **"Probabilistic
reasoning inside. Logical guardrails outside."** That is exactly the
architecture ADR-0029 reached for — enforcement that does not depend on
the model's cooperation, cannot be prompted away, and is testable as a
pure function. What ADR-0029 got wrong was *where the boundary falls*.

A boundary is drawn around the **probabilistic system**. The model is
inside it. Deterministic logic is outside. And so is **the person who
owns the machine** — they are on the same side as the guard, not the
thing it is aimed at.

By that reading, `Deny` with no override was a category error. Refusing
to let the *model* emit `rm -rf /` into a shell buffer is the principle
working. Refusing to let the *owner* lift that refusal on their own
hardware is a guardrail pointed at the wrong side of the line. Lisa's
entire thesis is that the machine is yours; a safety system that
overrides its owner contradicts the product it is protecting.

ADR-0029 half-anticipated this — it says "a `Deny` the user routinely
needs to work around is a `Deny` they will learn to disable" — and then
provided no way to work around it, which is the condition that produces
exactly that outcome.

## Decision

### 1. The principle, stated as a rule

**Guardrails sit between the model and the machine, never between the
owner and their machine.** Every enforcement point in this repo is
written for the first relationship. When one starts constraining the
person, it is misplaced and gets moved, not justified.

### 2. The invariant that keeps it rigorous

A principle about "outside" is worthless without a test for what counts
as outside. There is exactly one:

> **The boundary must not be reachable from inside.**

Apply it to any proposed guardrail: *can the probabilistic system touch
this?* If yes, it is not outside, whatever the diagram says. This is a
sharper tool than it first appears:

- A config file the human edits out-of-band **passes** — no tool call
  reaches it.
- A confirmation dialog the model can re-trigger until the human clicks
  yes **fails**. It is a boundary erodable from inside, which is why
  confirmation fatigue is a security property and not merely a UX
  complaint.
- Provenance that the **caller asserts** rather than the system deriving
  it **fails**, because an input the outside layer reasons over is
  writable from the inside. That reclassifies issue #55 from a missing
  check to a violation of the principle at the one point where it matters
  most.

### 3. The escape hatch: `lisa guard`

Relaxations live in `$XDG_CONFIG_HOME/lisa/guard-allow`, managed by
`lisa guard list | allow <rule> | forbid <rule>` and readable by anyone
who opens the file. Implemented in `libs/lisa-guard/src/overrides.rs`
and `cli/lisa/src/guard.rs`.

Three properties make it safe:

- **Unreachable from inside.** No tool, flag, argument or dialog reaches
  it. The forge jail confines writes to the project directory; the shell
  guard has no write surface at all. A retrieved document cannot talk the
  model into widening its own permissions, because there is no path.
- **Relaxed means warned, never silent.** A relaxed `Deny` becomes
  `Confirm`, carrying its original reason plus a note that it was
  relaxed. You asked for the block lifted, not for the action to go
  unmentioned.
- **Per rule, no wildcard.** `lisa guard allow` rejects unknown rule ids
  rather than accepting dead config that looks like it worked.

### 4. Who honours overrides

Only surfaces **with a human present**.

- `lisa suggest` honours them: the suggestion lands in your shell buffer
  and waits for your Enter, so a warning has somewhere to land.
- The **forge harness does not**. Nobody is watching that loop, so
  relaxing a rule there would delete the only check with no one to read
  the warning it degraded into. This asymmetry is deliberate and is
  enforced by `check_command` never consulting `Overrides`.

### 5. The ontology this depends on

The other half of that talk's argument is that a logical layer can only
reason over things that are **typed and declared** — you cannot write
deterministic rules about vibes. Lisa's formal surface already exists in
pieces and should be treated as one thing:

| declared surface | what the outside layer reasons over |
|---|---|
| the MCP manifest (§5.4, Appendix B) | which tools exist, what tier each carries, what undoes it |
| the provenance vocabulary (`tier.rs`) | where a piece of content came from, and whether that is trusted |
| the Ledger schema | what actually happened, append-only |
| `lisa-guard` rule ids | why an action was refused, stable enough to relax by name |

Growing a capability means growing this vocabulary *first*. An action the
ontology cannot describe is an action the guard cannot reason about.

## What was rejected

- **Leaving `Deny` absolute.** Defensible for a hosted product; wrong for
  an OS whose premise is that the hardware is yours.
- **A `--force` flag.** Reachable from inside the moment anything
  composes a command line, which is precisely the failure mode
  ADR-0029 round 3 found in `cargo --`.
- **A "yes, and don't ask again" button in the confirmation dialog.**
  The most natural UX and the worst answer: it moves the boundary inside,
  and it is reachable by a model that simply asks often enough.
- **A wildcard relaxation.** One typo away from disabling the guard, and
  it makes `lisa guard list` a lie.

## Consequences

- The owner has final authority over their own machine, and the model
  still cannot widen its own limits. Both halves are now true; before
  this ADR only the second was.
- Every future guardrail gets tested against "reachable from inside?"
  before it ships, which is a cheaper review than another adversarial
  round.
- #55 is promoted: caller-asserted provenance is not a nice-to-have fix,
  it is the principle being violated.
- New rule ids must be added to the catalog in `cli/lisa/src/guard.rs`,
  and a unit test fails if the guard emits a rule the catalog does not
  list — so a user can always relax a rule they actually saw.
- The forge loop's non-participation means an owner who wants a
  relaxation there has to say so explicitly in a future ADR, with a
  reason. That friction is intentional.
