# lisa-peer — who is calling

Spec: [ADR-0033](../../docs/adr/0033-identity-comes-from-the-transport.md).
Governing rule: [ADR-0030](../../docs/adr/0030-the-guardrail-boundary.md) —
*the boundary must not be reachable from inside*.

## What it does

Answers "who is calling" from the **transport**, never from the message.

Five independent adversarial reviews of `agentd`, the portal, `remoted`
and `contextd` returned forty issues and, without communicating, the same
sentence: *nothing in Lisa verifies who is calling*. `Confirm`/`Undo`
carried no identity at all, `actor` and provenance were read off the
wire, portal host identity came from `/proc/<pid>/comm` (which a process
sets itself), and memory namespaces were whatever the caller claimed.

That is one missing primitive, absent forty times. This is it.

Caller identity is the input every other check depends on — tiers,
grants, quotas, ACLs. When the caller supplies it, nothing downstream is
a boundary.

## How it works

Two mechanisms, deliberately separate because most callers need only the
first.

### Ownership — `PeerId` / `Owner`

*"Is this the same caller as before?"*

```rust
// When creating something a later call can act on:
let owner = Owner::of(PeerId::of(&header));

// When that later call arrives:
owner.require(&PeerId::of(&header))?;   // Err(NotTheOwner) otherwise
```

A unique bus name (`:1.42`) is assigned by the broker, never chosen by
the sender, unique per connection, and **never reused** after a
disconnect — so a caller cannot forge one or inherit a dead peer's. No
`/proc`, no credentials round-trip.

`PeerId::Direct` is a first-class case, not a fallback: on a p2p link
there is exactly one peer, so the connection *is* the identity. This
matters because the daemons' own tests run over zbus p2p — a primitive
that failed open there would be untested everywhere it is used.

### Credentials — `Peer` / `resolve`

*"Which user and which process?"* — only for decisions that depend on the
program itself ("may this caller mint grants for **other** apps?").

```rust
let peer = lisa_peer::resolve(&conn, &header).await?;
if !peer.is_same_user_as_us() { return Err(...); }
```

From the broker's `GetConnectionCredentials`, i.e. from the kernel.
Fails closed on an unknown uid.

### `exe_of_pid` — and why not `comm`

`/proc/<pid>/comm` is set by the process (`prctl(PR_SET_NAME)`, or just
`argv[0]`). `/proc/<pid>/exe` is a kernel-maintained link to the inode
actually executed and neither of those touches it. That is the difference
between asking the caller and asking the kernel.

## How to extend it

Adding an ownership check to a surface is three steps:

1. Store an `Owner` on whatever a *later* call can act on — a parked
   confirmation, a session object, a namespace.
2. `owner.require(&PeerId::of(&header))?` at the top of that later call.
3. Make the refusal indistinguishable from "does not exist". A
   wrong-owner error that differs from a not-found error is an oracle
   for what exists; see `BusError::NotYours` in `agentd`, which renders
   identically to `UnknownCall`.

Test it with two distinct `PeerId::Bus` values and assert the foreign one
is refused **and** that the rightful owner still works afterwards — a
refusal that evicts the entry is a denial-of-service dressed as a fix.

## Limits, stated rather than assumed

- **A pid is meaningful only while the peer is connected.** Pids are
  reused. Resolve at call time; never store one and re-resolve later.
- **`exe_of_pid` is Linux-only** and returns `Unsupported` elsewhere. A
  wrong answer about identity is worse than no answer, so there is no
  spoofable fallback — but it also means this path **cannot be tested on
  the macOS dev host** and needs a CI assertion.
- **This crate has had no adversarial review of its own.** Two fixes on
  2026-07-26 were later found wrong in a new way; a security primitive
  four daemons depend on deserves a round before it is extended to all
  of them.

## Adopters

`agentd` (confirmations, #93 — done). Remaining, tracked individually so
none is quietly skipped: portal grants and sessions (#107, #108),
`remoted`'s management plane (#99), `contextd` memory namespaces (#101),
`Undo` (#94, which also needs a tier check).
