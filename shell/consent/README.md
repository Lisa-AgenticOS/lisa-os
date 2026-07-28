# shell/consent — the desktop consent surface

Spec: issue #145, ADR-0035 §4, ADR-0030 (the guardrail boundary),
PLAN §5.10. Owns `dev.lisaos.Consent1` on the session bus.

## What it does

It shows the confirmation dialog for privileged Agent Bus calls, and it
calls `dev.lisaos.Agent1.Confirm` with the answer. That is all it does.

## Why it is its own process

The dialog used to live in `lisa-overlayd`, which also hosts the model.
So when the overlay routed a prompt to a privileged tool, the process
that **asked** for the call was also the process that **answered** for
the human — `_respond()` called `Confirm` on the same connection that had
made the `RequestCall`. agentd saw one peer with one unique name and
could not tell the two apart. The model was approving itself, with a
dialog drawn in between as decoration.

This is the pattern `xdg-desktop-portal` uses and the reason it uses it:
the thing that grants a capability must not be the thing that wants it.

agentd enforces the pairing rather than trusting the arrangement
(`may_answer` in `daemons/agentd/src/bus.rs`): owning the consent name
counts as oversight only when you are *not* the peer that asked. So even
if some future process ended up holding both roles, the approval is
refused rather than silently granted.

## How it works

```
overlay ──RequestCall──▶ agentd ──parks──▶ ConfirmationRequested (signal)
                                                    │
                                            lisa-consentd shows a dialog
                                                    │
                          agentd ◀──Confirm(id, approve)──┘
```

`agentd` resolves the answerer's identity from the broker — "who owns
`dev.lisaos.Consent1`?" — never from anything the message claims
(ADR-0033).

## What this must never grow

- **No model, no prompt entry, no tool calls of its own.** Its only
  inputs are agentd's signal and a human's click. The moment it can be
  driven by generated text it stops being a second pair of eyes
  (ADR-0030: anything reachable from inside is not a guardrail).
- **No `Approve()` D-Bus method.** It deliberately exposes only `Ping`
  and `PendingCount`. A peer that could ask this daemon to approve
  something could launder its own request through it, which is the hole
  being closed. The only approver is the pointer.

## Details that are deliberate

- **Closing the window denies.** A dismissed dialog must not leave a
  privileged call parked until its TTL, where it looks to the user like
  the action is still going to happen.
- **Escalated calls say so, prominently.** When rule-6 provenance
  escalated the call, the dialog says the action was suggested by content
  from outside this machine — the single most important fact on the
  dialog, so it is not buried under the arguments.
- **Arguments render as compact JSON, not prose.** A sentence built from
  attacker-influenced values reads as if Lisa is recommending it; a
  monospace blob reads as data, which is what it is.
- **Losing the bus name exits.** Two dialogs for one call is worse than
  one, and agentd only trusts whoever owns the name.

## How to extend it

- **A queue.** Today each parked call gets its own window. Several at
  once will stack; a list would be better, and needs a design that cannot
  be used to bury a destructive call under noise.
- **Remembered answers** would be a grant (ADR-0036 §4), not a checkbox
  here: visible, listable, revocable in one place, and ledgered on every
  use.

## Limits

- **Not yet packaged or wired.** `os/packages/lisa` does not install it,
  and `lisa-overlayd` still calls `Confirm` itself for chips. Until both
  land, a destructive call originated by the overlay is refused with
  `NeedsConsentSurface` rather than approved — failing closed on a real
  hole, but a visible behaviour change.
- **Untested against a live agentd.** The logic in `bus.rs` has unit
  tests and a mutation check; this process has been read, not run.
- **No tests of its own.** The dialog is GTK and the parsing is small;
  `describe()` is the part worth a test and does not have one yet.
