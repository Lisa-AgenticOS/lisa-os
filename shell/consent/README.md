# shell/consent — the desktop consent surface

Spec: issues #145, #244, #251, #289; ADR-0035 §4, ADR-0030 (the guardrail
boundary), ADR-0033 (identity comes from the transport), PLAN §5.10.
Owns `dev.lisaos.Consent1` on the session bus.

## What it does

It shows the confirmation dialog for privileged Agent Bus calls, and it
calls `dev.lisaos.Agent1.Confirm` with the answer. It also shows the
**refusal report** for calls agentd would not run at all — a dialog with
one button and nothing that approves. That is all it does.

## Two processes, and which one is the peer

```
shell/consent/daemon/      Rust. /usr/bin/lisa-consentd.
                           Owns dev.lisaos.Consent1, subscribes to
                           agentd's signals, makes the Confirm call.
                           NEVER draws anything.

shell/consent/lisa-consentd.js
                           GJS/GTK4. The window. Spawned as a CHILD of
                           the above, with no session bus address.
                           NEVER touches the bus.
```

Until 2026-08-06 there was one process and it was the GJS file. That
worked, and it left #289 open. `Exec=/usr/bin/lisa-app consent/lisa-consentd.js`
ends in `exec gjs -m "$found"`, so the kernel's answer for
`/proc/<pid>/exe` on the process owning the consent name was
`/usr/bin/gjs-console` — measured on the reference iMac, pid 18669.
agentd's program allowlist therefore had to contain an **interpreter**,
and an interpreter on an allowlist authorises every program that
interpreter can run:

1. the model's host parks a destructive call;
2. it `fork()`s and `exec`s `gjs` on a script of its own — the child has
   a fresh pid, so `consent.same_process` does not fire, and a fresh
   connection, so `consent.self_approval` does not either;
3. the child takes `dev.lisaos.Consent1` (`session.conf` ships
   `<allow own="*"/>`, and the name is normally unowned because it is
   activatable);
4. `/proc/<child>/exe` is `/usr/bin/gjs-console`, which is on the
   allowlist;
5. agentd calls it the human's dialog and runs the call.

Re-run on the device on 2026-08-06 against a private bus: the forked
child reported `exe /usr/bin/gjs-console` — byte-identical to the live
consent surface's — and took the name.

**A native launcher that `exec`s the GJS surface changes nothing**:
after `execve` the exe is `gjs` again. The process that owns the name has
to be the binary. So it is.

## How it works

```
overlay ──RequestCall──▶ agentd ──starts /usr/bin/lisa-consentd if
                                   nothing owns dev.lisaos.Consent1
                                 ──parks──▶ ConfirmationRequested (signal)
                                                    │
                                        lisa-consentd (Rust, owns the name)
                                                    │  JSON lines on a pipe
                                                    ▼
                                        gjs lisa-consentd.js (no bus)
                                                    │  a person clicks
                                                    ▼
                          agentd ◀──Confirm(id, approve)── lisa-consentd
```

The private channel is one JSON object per line, both directions
(`daemon/src/protocol.rs`):

```
in   {"kind":"confirm","call_id":41,"spec":"<agentd's json>"}
in   {"kind":"refusal","call_id":41,"report":"<agentd's json>"}
out  {"call_id":41,"answer":"allow"|"deny"|"dismiss"}
```

The call id travels out and comes back so the daemon can match an answer
to a dialog. It is not a capability: the renderer has no bus, and the
daemon drops an answer for a dialog it did not open.

`agentd` resolves the answerer's identity from the broker and the kernel
— "who owns `dev.lisaos.Consent1`?" and "what is that connection's
`/proc/<pid>/exe`?" — never from anything the message claims (ADR-0033).
Owning the name says a peer *asked* to be the dialog; the exe says what
it *is*, and `bus.rs::may_answer` requires both plus "not the requester's
process".

Nothing else on the machine ever calls a method on this name, so nothing
else could ever start it: `GetNameOwner` does not activate, and a signal
activates nothing at all. That is why it was packaged, activatable and
never once running on a real device (#244) — agentd now activates it on
the one event that needs it, a destructive call parking.

The bus name is claimed **last**, after the object is served and the
signal match rule is in place, because agentd treats "the name is owned"
as "the dialog is listening" and emits immediately afterwards. And a
dialog is the *only* way a destructive call can be approved: if this
daemon will not start, agentd refuses the approval and ledgers the
refusal rather than letting the requester answer for itself.

## Three properties of the split that are load-bearing

1. **The renderer has no session bus.** `DBUS_SESSION_BUS_ADDRESS` and
   `DBUS_STARTER_*` are removed from the child's environment
   (`daemon/src/renderer.rs`). It cannot own a name, cannot call
   `Confirm`, and cannot be mistaken for either. Verified on the device:
   the child holds no fd on `/run/user/1000/bus`.
2. **The dialog file is pinned to `/usr`, not the app channel.** Every
   other Lisa surface launches through `lisa-app`, which prefers
   whatever `lisa apps update` unpacked under `/var/lib/lisa-apps` — a
   directory anything running as the user can write. The live surface on
   the reference device was running exactly that copy
   (`/var/lib/lisa-apps/payloads/shell/current/consent/lisa-consentd.js`).
   A guardrail cannot be updated that way (ADR-0030 §2: *is it reachable
   from inside?*), so this one surface gives up ADR-0020 and ships with
   the package. Changing the dialog now needs a package update.
3. **The accessibility bus is handed over deliberately.** Stripping the
   session bus would otherwise leave the most important dialog on the
   machine the one dialog a screen reader cannot read. The daemon
   resolves `org.a11y.Bus.GetAddress` on its own connection and passes
   `AT_SPI_BUS_ADDRESS` down. A guardrail sits between the model and the
   machine, never between a person and their own machine (ADR-0030 §1),
   and a blind owner locked out of their own consent decision is the
   second kind.

## The refusal report (#251)

`RefusalReported` is a second signal, and it is separate from
`ConfirmationRequested` so that this surface cannot mistake a refusal for
something to draw an Allow button on. There is no parked call behind it,
so there is nothing `Confirm` could answer even if this file tried — and
now there is a third layer saying so: the renderer answers `dismiss`, and
`protocol::confirm_for(Dismiss)` is `None`, so no `Confirm` is reachable
from that path at all.

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
- **No environment variable that names the dialog file.** The dev
  override (`LISA_CONSENT_RENDERER_DEV`) is compiled out of a release
  build, which is what the package produces. Anything that can set the
  daemon's environment could otherwise name its own dialog, and a dialog
  you wrote approves what you like.

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
- **The name is requested `DoNotQueue` and without `AllowReplacement`.**
  A second consent surface fails loudly rather than sitting in a queue
  looking like it is running, and nothing can take the name away once we
  hold it.
- **The signal match rule filters on the sender.** The broker resolves
  `dev.lisaos.Agent1` to its current owner, so a peer that merely emits a
  signal with the right interface cannot put a fabricated confirmation on
  screen and collect a click for a real parked call.

## How to extend it

- **A queue.** Today each parked call gets its own window. Several at
  once will stack; a list would be better, and needs a design that cannot
  be used to bury a destructive call under noise.
- **Remembered answers** would be a grant (ADR-0036 §4), not a checkbox
  here: visible, listable, revocable in one place, and ledgered on every
  use.

## Limits

- **#289 is not closed by this package alone.** `CONSENT_SURFACE_PROGRAMS`
  in `daemons/agentd/src/dbus.rs` is a *disjunction*, and it still reads
  `["/usr/bin/lisa-consentd", "/usr/bin/gjs"]`. While the interpreter
  entry is there, the fork-and-exec-gjs attack above still works —
  shipping the binary is necessary and not sufficient. The remaining
  change is one line, and
  `tests/injection-suite/tests/consent_program.rs::the_shipped_allowlist_still_admits_an_interpreter`
  goes red the moment it lands, which is how it is meant to be found.
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
- **Still not clicked, on a device.** The renderer has been run on the
  reference iMac with no session bus and the real display: it draws, it
  takes a `confirm` message, and it exits 0 when the pipe closes. The
  Allow/Deny buttons themselves have still never been driven by an
  automated test. The obvious driver — at-spi — is not available: the
  device's accessibility registry reports **zero** applications, so a
  script cannot find the button. That is worth knowing on its own, since
  it means nobody has confirmed a screen reader can read this dialog
  either.
- **The GJS dialog is still staged into the app-channel payload**
  (`os/repo-tools/build-apps-payload.sh` lists `consent` in
  `ap_surfaces`), so `/var/lib/lisa-apps/.../consent/lisa-consentd.js`
  will exist and be unread. Harmless — the daemon only ever opens the
  `/usr` copy — but it is a second spelling of a file with one meaning,
  and it should be dropped from that list.
- **`describe()` has no test.** The dialog is GTK and the parsing is
  small; the parsing is still the part worth a test and still does not
  have one. The Rust side of the channel is tested
  (`daemon/src/protocol.rs`); the JavaScript side is not.
