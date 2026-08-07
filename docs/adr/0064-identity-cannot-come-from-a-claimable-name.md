# ADR-0064 — identity cannot come from a claimable name: harnessd leaves the user namespace

- **Status:** proposed
- **Date:** 2026-08-07
- **Trigger:** the adversarial close-replay of #306. The fix under
  review moved the trust root one hop instead of removing it, and the
  replacement root is squattable on the reference device *today*.
- **Amends:** ADR-0033 (identity comes from the transport) — by naming
  the one place the repo violated it while believing it did not.
- **Bears on:** ADR-0029/0030 (guardrails outside the model), #161
  (mount sandboxing in user units implies a user namespace).
- **Claims:**
  - `path:daemons/harnessd/src/caller.rs` — the identity decision this changes
  - `path:os/packages/lisa/lisa-harnessd.service` — the unit whose options force the workaround
  - `path:os/repo-tools/check-user-units.py` — the gate whose ALLOWED entry encodes the old choice

## Context

harnessd decides one thing that matters: whether a caller is **the
person's prompt surface** (`Trigger::Prompt` — file tools, write-tier
bus calls, `user` provenance, memory) or **an event source**
(`Trigger::Event`). #306 showed the first version of that decision
resting on a first-come bus name: `app.lisaos.Assistant` is
D-Bus-activatable, unowned whenever the Assistant is closed, and
`session.conf` ships `<allow own="*"/>`.

227ad9c added a second fact — "does that connection run a prompt-surface
program" — and asked **agentd** for it, because harnessd cannot read
`/proc/<peer>/exe` from inside the user namespace its own unit implies
(#161). The question travels to well-known name `dev.lisaos.Agent1`,
and harnessd believes the reply.

On the reference device, `dev.lisaos.Agent1` was **unowned and not
activatable** (agentd had zombied — #347). Under `<allow own="*"/>` a
squatter takes `dev.lisaos.Agent1`, then `app.lisaos.Assistant`, and
answers `true` about itself. Two `RequestName` calls buy the whole
escalation the original issue described. The oracle guarding the name
is reachable through a name.

`check-user-units.py`'s ALLOWED entry states the old choice honestly:
harnessd keeps `ProtectHome`/`ProtectSystem`/`PrivateDevices`, and *"the
identity mechanism is chosen to survive them"*. That is the sentence
this ADR retires: an identity mechanism chosen to fit a sandbox is a
mechanism chosen for the wrong reason, and it produced one that a peer
can answer for itself.

## Options

**A — harnessd leaves the namespace and asks the kernel.** Drop the
mount-class options from its unit (it leaves `check-user-units.py`'s
ALLOWED list, which the list already demands of anything wanting
`exe_of_peer`), and let harnessd resolve `/proc/<peer>/exe` itself. The
agentd oracle and its 5-second deadline are deleted, not hardened.

**B — keep the sandbox, make the oracle unforgeable.** Replace the bus
query with a systemd-passed socket whose other end is fixed at start-up.
Rejected: the threat model is a process running **as the user**, and
every socket path the two daemons can share sits in a user-writable
runtime directory. A hostile peer can unlink and re-bind it. This
substitutes a claimable path for a claimable name.

**C — status quo plus a liveness fix.** Fix #347 so `dev.lisaos.Agent1`
is rarely unowned. Rejected as a *fix*: it narrows the window rather
than closing it, and a window that only opens when a daemon dies is one
an attacker can encourage.

## Decision

**Option A.** Identity comes from the transport and the kernel, never
from a name any peer can claim (ADR-0033, rule 6b). harnessd asks the
kernel about its callers, which means it may not run in a namespace that
hides them.

## Consequences and the cost, stated plainly

- harnessd loses `ProtectHome=read-only`, `ProtectSystem=strict` and
  `PrivateDevices=yes`. That is a real loss and must not be waved
  through: harnessd is the process the model runs inside, which is
  ADR-0029's whole subject.
- What actually holds after the loss: the model reaches the machine
  through **tools**, not through harnessd's own file access — the bus
  enforces tiers and provenance escalation in code (ADR-0036 §3,
  #302), the shell and command tools are Landlock-confined
  (#307/#309), and `IPAddressDeny=any` + `RestrictAddressFamilies=AF_UNIX`
  are seccomp/IP-level and survive the change. The dropped options
  bound *harnessd's own* filesystem view; the guardrails that bound the
  **model** are elsewhere, by design.
- `NoNewPrivileges`, the address-family restriction and `IPAddressDeny`
  stay. Only the mount-class options go.
- `check-user-units.py` loses its `lisa-harnessd.service` entry — the
  gate then enforces this decision rather than exempting it.
- ADR-0033's rule gains a worked example: the violation hid behind a
  *second* fact, so "we check two things" is not the test. The test is
  whether any of them can be answered by the caller.

## Limits

- Not implemented. This records the decision and its cost so the
  implementation is reviewed as one change, with the sandbox loss
  visible rather than buried in a commit that says "fix #306".
- Until it lands, #306 stays open and the device stays exposed to a
  same-user squatter. #347 (agentd exiting rather than zombieing)
  narrows the window and has landed; it is mitigation, not the fix.
- `PROMPT_SURFACE_PROGRAMS` still resolves to `{gjs-console}` because
  `/usr/bin/lisa-assistant` does not exist — so even after this change,
  the program half authorises any GJS script. Shipping that binary is a
  separate, necessary step (the consent surface got its own,
  `/usr/bin/lisa-consentd`, in #289).
