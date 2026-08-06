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

Three mechanisms, deliberately separate because most callers need only
the first.

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

**`Owner::allows` compares connections, not processes.** A `false` means
"some other socket" and nothing more, because one process may hold as
many connections as it likes. Anything that reads a `false` here as
"somebody independent is involved" needs `Process` below — that reading
is #289.

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

`resolve_unique_name` answers the same question about a **third**
connection rather than about the sender, for the one case where the
daemon that must decide cannot look: `harnessd` lives in a private user
namespace and every peer's `/proc/<pid>/exe` is EACCES there (#161), so
`agentd` answers on its behalf (#306). Unique names only — a well-known
name's owner can change between the question and the answer.

### The process — `Process` / `same_process`

*"Are these two connections the same running process?"*

```rust
// At park time, held for as long as the object lives:
let requester = lisa_peer::Process::of_peer(&peer)?;
// When somebody else tries to act on it:
lisa_peer::same_process(answerer.as_ref(), Some(&requester));
```

Built from the broker's pidfd, which **pins its pid**: the kernel will
not recycle the number while the descriptor is open, so a `Process` is
safe to hold across a parked confirmation — unlike a bare pid, which is
the reuse window ADR-0033 warns about.

This exists because `PeerId` was being asked to answer it and cannot.
`agentd` refused a model host's self-approval by comparing unique names,
and the same process answered from a second socket while owning the
consent surface's name; `session.conf` ships `<allow own="*"/>`, so
taking that name costs nothing (#289).

Same pid is the same process. **Different pid is not necessarily a
different program** — `fork()` — which is why this is always one of two
checks and never the only one; the other is `exe_of_peer` against an
allowlist.

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
  reused. Resolve at call time; never store one and re-resolve later —
  or hold the pidfd, which is what `Process` does and the only reason it
  is allowed to outlive the call that created it.
- **`Process` cannot tell a `fork()` apart.** A child gets a new pid
  running the same executable, so a process that forks before opening a
  second connection is a different `Process`. The program allowlist is
  the check that has to catch that, and it only catches it when the
  program is a binary of ours rather than an interpreter (#289, #306).
- **`exe_of_pid` is Linux-only** and returns `Unsupported` elsewhere. A
  wrong answer about identity is worse than no answer, so there is no
  spoofable fallback — but it also means this path **cannot be tested on
  the macOS dev host** and needs a CI assertion.
- **This crate has had no adversarial review of its own.** Two fixes on
  2026-07-26 were later found wrong in a new way; a security primitive
  four daemons depend on deserves a round before it is extended to all
  of them.

## Adopters

`agentd` (confirmations, #93 — done; process identity for #289),
`harnessd` (the trigger ceiling, via agentd for #306). Remaining, tracked individually so
none is quietly skipped: portal grants and sessions (#107, #108),
`remoted`'s management plane (#99), `contextd` memory namespaces (#101),
`Undo` (#94, which also needs a tier check).
