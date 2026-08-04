# shell/consent — the desktop consent surface

Spec: issue #145, ADR-0035 §4, ADR-0030 (the guardrail boundary),
PLAN §5.10. Owns `dev.lisaos.Consent1` on the session bus.

## What it does

It shows the confirmation dialog for privileged Agent Bus calls, and it
calls `dev.lisaos.Agent1.Confirm` with the answer. It also shows the
**refusal report** for calls agentd would not run at all — a dialog with
one button and nothing that approves. That is all it does.

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
overlay ──RequestCall──▶ agentd ──starts this daemon if nothing owns
                                   dev.lisaos.Consent1 (D-Bus activation)
                                 ──parks──▶ ConfirmationRequested (signal)
                                                    │
                                            lisa-consentd shows a dialog
                                                    │
                          agentd ◀──Confirm(id, approve)──┘
```

Nothing else on the machine ever calls a method on this name, so nothing
else could ever start it: `GetNameOwner` does not activate, and a signal
activates nothing at all. That is why it was packaged, activatable and
never once running on a real device (#244) — agentd now activates it on
the one event that needs it, a destructive call parking.

Two consequences for this file. The bus name is claimed **last**, after
the signal subscription is in place, because agentd treats "the name is
owned" as "the dialog is listening" and emits immediately afterwards.
And a dialog is the *only* way a destructive call can be approved now: if
this daemon will not start, agentd refuses the approval and ledgers the
refusal rather than letting the requester answer for itself.

`agentd` resolves the answerer's identity from the broker — "who owns
`dev.lisaos.Consent1`?" — never from anything the message claims
(ADR-0033).

## The refusal report (#251)

`RefusalReported` is a second signal, and it is separate from
`ConfirmationRequested` so that this surface cannot mistake a refusal for
something to draw an Allow button on. There is no parked call behind it,
so there is nothing `Confirm` could answer even if this file tried.

```
Refused — this is not something Lisa will do
app.lisaos.Probe244 asked to do this, and it was not done.
`/` is the system, or a whole home directory. …
This was suggested by content from outside this machine.
If you genuinely want this, do it yourself in a terminal.
                                                     [ OK ]
```

Three properties this window has to keep:

1. **Nothing in it performs, composes or copies the refused action.** No
   copy-to-clipboard of the target, no "fix this", no deep link into
   Settings with a loosening entry pre-filled. The reason label is
   deliberately *not* `selectable`, unlike the argument dump on the
   confirmation dialog. The friction is the safety; removing it rebuilds
   the click-through with extra steps.
2. **It reports rather than asks.** One button, and dismissing it changes
   nothing — there is no state to change.
3. **It must stay rare.** The justification for putting a refusal on
   screen at all is that the owner should learn immediately that outside
   content tried to destroy their system. That collapses if these become
   common, at which point they train dismissal exactly as Allow dialogs
   do. How often this window appears is a correctness signal for the
   guard catalogue, not just an annoyance.

An *out-of-scope* refusal (`No`, not `HardNo`) names the scope that would
permit it — as a sentence. Widening happens in Settings (#253), reached
deliberately, never from this window: `~/.local/share/lisa/` holds
`ledger.db` and `grants.db`, and one "always allow" there would let an
agent erase its own audit trail and edit its own grants.

Also from #251: **Deny holds focus** on the confirmation dialog. If Enter
activated Allow, a destructive action would be one keystroke from a
person who was still typing when the dialog appeared.

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

- **A headless host cannot approve a destructive call at all.** The only
  approver is this dialog, and this dialog needs a display: on a machine
  with a session bus and no seat, a destructive call can be parked and
  withdrawn but never approved. That is deliberate (#244) — the previous
  behaviour was to let the requester approve itself, which on the
  reference desktop meant the model's own connection. A headless
  approver would need a path of its own, on a connection that is not the
  requester's, and it does not exist yet.
- **`lisa-overlayd` still calls `Confirm` itself for chips.** Write-tier
  calls are approved by the app that drew the chip, so for a chip agentd
  cannot tell "a person clicked" from "the process decided". Destructive
  calls are the ones fenced.
- **Run, but not clicked, against a live agentd.** On the reference iMac
  agentd activated this daemon, it took the name, and `PendingCount`
  reported the dialog up while the requester's self-approval was refused
  — and an independent surface's approval went through. Nobody has yet
  driven the Allow/Deny buttons in an automated test.
- **No tests of its own.** The dialog is GTK and the parsing is small;
  `describe()` is the part worth a test and does not have one yet.
