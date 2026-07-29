# ADR-0033: Identity comes from the transport, not the message

- Status: accepted (primitive implemented + first application 2026-07-26;
  remaining surfaces tracked as issues)
- Date: 2026-07-26
- Relates: ADR-0030 (the guardrail boundary), PLAN §5.4, §5.5, §5.10,
  Appendix C; issues #55, #56, #93, #94, #97, #99, #100, #101, #106,
  #107, #108
- Implements: `libs/lisa-peer`

## Context

Five adversarial reviews ran the same afternoon against four components —
`agentd`, `xdg-desktop-portal-lisa`, `remoted`, `contextd` — plus a
fourth round on `lisa-guard`. The reviewers did not communicate. They
returned forty issues and, independently, the same sentence:

> **Nothing in Lisa verifies who is calling.**

The specific findings:

| surface | what identity was | consequence |
|---|---|---|
| `agentd` `RequestCall` | `actor` and `provenance` read off the wire (#55, #56) | the trust chain the tier machinery reasons over is attacker-chosen; the Ledger's "who" is a label |
| `agentd` `Confirm` | **nothing at all** (#93) | call ids are sequential from 1, so any peer sweeps the range and releases somebody else's parked privileged call — *including ahead of the human*, on a modal-tier call with an untrusted chain |
| `agentd` `Undo` | **nothing at all** (#94) | a no-argument method that dispatches destructive-tier tools, unconfirmed, attributed to `host` |
| portal host callers | `/proc/<pid>/comm` (#106) | a process sets its own `comm`; rename the binary, inherit the victim's grants and quota |
| portal `Grant()` | caller unauthenticated (#107) | any host process mints grants for any app — a consent bypass |
| portal sessions | not bound to the opener (#108) | any app cancels, closes or drives a victim's session and bills their quota |
| `remoted` management | caller unauthenticated (#99) | any session peer flips every egress scope and overwrites credentials |
| `contextd` `Search` | scopes supplied by the caller (#100) | the ACL is advisory; the shipping overlay omits scopes entirely |
| `contextd` memory | namespace asserted (#101) | any peer lists or wipes another app's namespace |

That is not forty bugs. It is **one missing primitive, absent forty
times**.

ADR-0030 states the invariant: *the boundary must not be reachable from
inside*. Caller identity is the input every other check depends on —
tiers, grants, quotas, ownership, ACLs. When the caller supplies it,
nothing downstream is a boundary. The `agentd` reviewer put it best:

> A pure function over attacker-controlled inputs is not a boundary.

Tier resolution itself survived brute-forcing (every chain of length 0–2
over 18 adversarial provenance strings; escalation never inverts, only
the exact bytes `user` are trusted, homoglyphs and trailing NULs fail
closed). The logic was never the problem. Its inputs were.

## Decision

**Identity is whatever the transport reports, never what the message
claims.** Two mechanisms, deliberately separate because most callers need
only the cheaper one.

### 1. Ownership — `PeerId` / `Owner`

Answers *"is this the same caller as before?"*.

A unique bus name (`:1.42`) is assigned by the message broker, is never
chosen by the sender, is unique for the connection's lifetime, and is
**never reused** after a disconnect. A caller cannot forge one and cannot
inherit a dead peer's.

Store an `Owner` alongside anything a *later* call may act on — a parked
confirmation, a portal session, a memory namespace — and check it before
acting. That one discipline closes #93, #101 and #108.

`PeerId::Direct` models a point-to-point connection as a first-class
case, not a fallback: on p2p there is exactly one peer, so the connection
*is* the identity. This matters because the daemons' own tests run over
zbus p2p, and a primitive that failed open there would be untested
everywhere it is used.

### 2. Credentials — `Peer` / `resolve`

Answers *"which user and which process?"*, from the broker's
`GetConnectionCredentials` (i.e. from the kernel via `SO_PEERCRED`), for
the few decisions that depend on the program itself: "may this caller
mint grants for *other* apps" (#107), "may this caller flip every egress
scope" (#99). Fails closed on an unknown uid.

### 3. `exe_of_pid`, and why not `comm`

`/proc/<pid>/comm` is set by the process: `prctl(PR_SET_NAME)`, or simply
being exec'd with a chosen `argv[0]`. `/proc/<pid>/exe` is a
kernel-maintained link to the inode actually executed; neither of those
touches it. That is the difference between asking the caller and asking
the kernel, and it is the fix for #106.

Non-Linux returns `Unsupported` rather than a fallback: **a wrong answer
about identity is worse than no answer.**

Two limits stated rather than wished away: a pid is meaningful only while
the peer is connected (pids are reused — resolve at call time, never
store), and the executable may have been replaced since exec (Linux
appends `" (deleted)"`, which is reported, not accepted).

### 4. Refusals must not become oracles

`BusError::NotYours` renders identically to `UnknownCall`. A sweep must
not be able to use the error to learn which ids are live — otherwise the
fix hands the attacker the reconnaissance for the next attempt.

## What was rejected

- **Trusting `actor`/`app_id` "until the portal attaches real identity"**
  (the status quo, per the comment in `CallRequest`). The portal's own
  host identity turned out to be spoofable, so the thing being waited for
  would not have fixed it.
- **uid-only checks.** The session daemons serve one user; the threat is
  another *process* of that same user — an app you installed, or an app
  the agent built. uid is necessary and nowhere near sufficient.
- **A capability token in the message.** It would be one more thing the
  message asserts, which is the bug.
- **Fixing each surface locally.** Forty patches, forty chances to differ.

## Consequences

- Every D-Bus surface gains a cheap ownership check; the ones making
  program-level decisions additionally resolve credentials.
- `PeerId::Direct` keeps the existing p2p test suites meaningful instead
  of forcing a bus daemon into unit tests.
- **`exe_of_pid` cannot be tested on the macOS dev host.** Its Linux
  behaviour is exercised in CI only, and this is exactly the class of
  gap that produced the initrd and preset regressions — so the portal's
  adoption of it needs a CI assertion, not a local run.
- The remaining surfaces are now mechanical: store an `Owner`, check it.
  They are tracked individually so none is quietly skipped.
- One honest caveat: this primitive has had **no adversarial review of its
  own**. Two of today's fixes were later found wrong in a new way, and a
  security primitive adopted by four daemons deserves a round before it
  is extended to all of them.

## Adoption: the portal (#106, #107, #108)

The portal was the surface this ADR was written from, and it is now on
the primitive. Three things came out of doing it that the ADR did not
anticipate:

1. **`exe` alone was not enough.** Swapping `comm` for `exe` and keeping
   the basename comparison would have moved the forgery rather than
   removed it: a process that cannot set its `comm` can still name a
   file. Host identity matches the **whole resolved path** against an
   installed `.desktop` entry, and resolves bare `Exec=` names from a
   fixed system list rather than `$PATH`.

2. **The p2p suites stayed meaningful, and that was the problem.**
   `PeerId::Direct` let the existing tests keep running — but it also
   means a p2p suite *cannot fail* an ownership test, because there is
   only one peer to be. The portal's ownership fix needed a test file
   that starts a real `dbus-daemon` and connects two clients
   (`portals/xdg-desktop-portal-lisa/tests/bus.rs`). `Direct` keeping
   old tests green is a convenience for unrelated coverage, never
   evidence about ownership.

3. **The CI assertion this ADR asked for exists**, and it needed a
   second mechanism: the bus tests skip where `dbus-daemon` cannot be
   started (Homebrew's macOS build does not print its address), so
   `LISA_REQUIRE_BUS_TESTS=1` makes skipping fatal and CI sets it. A
   test that skips silently is the same failure as a test that does not
   exist.

The credentials half (`Peer` + `exe_of_peer`) answers only #107 —
"may this caller mint grants for other apps" — via an allowlist of
shipped executables. Everything else the portal needed was ownership.
