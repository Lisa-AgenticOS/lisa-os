# ADR-0059 — `lisa-remoted` brokers model egress, not every socket on the machine

- **Status:** accepted, partially executed — the reasoning and the
  exemption are recorded here and cited from
  `os/repo-tools/check-egress-units.py`. The remaining edit is CLAUDE.md
  rule 5's own wording, quoted verbatim in "The wording rule 5 should
  carry" below; until that lands, the operating manual still states the
  absolute this record retires.
- **Date:** 2026-08-06
- **Claims:**
  - `path:os/packages/lisa/lisa-mail-sync.service` — the unit this is about
  - `symbol:ADR-0059@os/repo-tools/check-egress-units.py` — the gate cites this record instead of arguing in a justification string
  - `nomatch:it is not something a unit-file assertion can fix@os/repo-tools/check-egress-units.py` — the old prose, which recorded the discrepancy where only a lint tool could see it

## Context

CLAUDE.md rule 5 says `lisa-remoted` is the **sole** egress broker.
`lisa-mail-sync.service` runs `lisa mail sync`, which runs `mbsync`,
which opens a TCP connection to the user's IMAP server. The two
statements cannot both be read literally, and until now the only place
that said so was a justification string inside
`check-egress-units.py` — a lint tool, read by the gate and by nobody
else (#286, split out of #285).

That is the actual defect. A rule that a shipped unit visibly
contradicts, with the contradiction recorded only where a checker
happens to print it, is a rule that has already stopped being one.

Two honest answers were on the table.

## Considered: route mail sync through the broker

Consistent with rule 5 as written. It fails on what `lisa-remoted`
*is*.

`remoted` is not a socket. It is a **request broker with an opinion**:
a provider registry, one credential per provider in a 0700 state dir,
per-scope "may offload" switches that all default off, and a
`remote.generate` ledger entry that must land *before* the first byte
leaves. Every one of those is meaningful because remoted can see the
request — it parses a JSON body, knows which scope's data is in it, and
can refuse.

IMAP gives it none of that. `mbsync` is a third-party binary that opens
its own long-lived, stateful, TLS connection. Routing it through the
broker means one of:

- **A CONNECT tunnel.** remoted would forward bytes to `host:port` and
  learn nothing. The ledger entry becomes "bytes went somewhere", the
  scope switches have no content to be about, and the component that
  holds every provider credential gains a *generic egress primitive* —
  anything that can reach remoted's unix socket can now reach any host.
  That is strictly worse than the status quo: we would have paid the
  whole cost of the plumbing to make the one door out into a door for
  everyone.
- **Reimplementing IMAP, IDLE and SMTP submission inside remoted.**
  Thousands of lines of mail client, and it puts the user's *mail* in
  the same process as every provider API key. Least privilege runs the
  other way. It also contradicts rule 4: mbsync is the boring choice,
  and a hand-written IMAP client is the opposite of one.

Neither buys the property rule 5 exists to protect, and both enlarge
the attack surface of the only component that has network access.

## Decision

**Mail sync is exempt, and rule 5 is stated as what it means.**

Rule 5 is about **model traffic**, and about **daemons that hold the
user's context leaving with it**. Read ADR-0010 and PLAN §4 dataflow
rule 2 together and that is the whole of it: `inferenced` runs the
model, `contextd` holds the consented index, `agentd` dispatches tools
— none of them may open a route off the machine, so that every request
made *on the user's behalf by the model* passes one gate that can
refuse it, price it and log it.

Mail sync is a different shape on every axis that matters:

| | model offload (remoted's job) | mail sync |
|---|---|---|
| destination | a provider **we** listed | the user's own server, from their own Online Accounts entry |
| direction | the user's context goes **out** | mail comes **in**; what leaves is IMAP commands |
| credential | a system-held provider key | the user's own token, from their own session's keyring |
| consent | must be granted per scope, default off | already granted, out of band, by connecting the account |

A mail client contacting the user's own mail server on the user's own
credential is not the thing rule 5 prevents. Nothing about the user's
context leaves; the mail is already on that server. By the same
reasoning nobody has ever proposed routing Surfer, `pacman -Syu`, NTP
or GOA itself through the broker — and rule 5 was never meant to, which
is exactly why stating it as an absolute made it false.

**The exemption is scoped, and here is what revokes it.** Fetching is
exempt. *Sending on the model's behalf is not*: the day an agent
composes a message and a tool sends it, that outbound content is model
traffic, and it belongs behind remoted's per-scope consent and a
`remote.` ledger entry like any other offload. The line is not
"mail is special"; it is "the model's output leaving the machine is
always brokered."

### Known gap, stated rather than closed

Mail sync produces **no ledger entry**. The `remote.` prefix is the
machine-readable "leaves your hardware" marking and mail sync carries
none, so "what left this machine today" is answerable for model traffic
and not for mail. That is a real hole in the audit story and it is not
what this record closes — it closes the question of whether the traffic
should be *brokered*. A `mail.sync` ledger kind, written by the CLI
around the mbsync run, is the cheap fix and needs no broker at all.

## The wording rule 5 should carry

Replacing the current absolute. This is the remaining edit; the ADR is
not a substitute for it, because the operating manual is where people
read the rule.

> 5. **Egress is architecture.** `lisa-inferenced`, `lisa-contextd` and
>    `lisa-agentd` never get network access, and **every request made on
>    the user's behalf by the model goes through `lisa-remoted`** — the
>    sole broker for model egress, gating each one on per-scope consent,
>    a stored credential and a `remote.` ledger entry before the first
>    byte leaves (ADR-0010, PLAN §4 dataflow rule 2). Never add a network
>    dependency to a no-egress daemon.
>
>    This is not "nothing else opens a socket", and never was: `pacman`,
>    the browser, Online Accounts and `lisa mail sync` all reach the
>    network directly, on the user's own behalf and to hosts the user
>    chose. Mail sync's exemption is argued in ADR-0059 — and its limit
>    is there too: sending content the model composed is model traffic
>    and is brokered like any other.

## Consequences

- The discrepancy stops living in a lint tool's justification string.
  `check-egress-units.py` cites this record; the argument is here.
- `lisa-mail-sync.service` needs no change. It already restricts what it
  can reach on this machine, and what it reaches off the machine is the
  user's own server.
- The rule becomes checkable in the direction that matters: a *daemon*
  gaining network access still fails `check-egress-units.py`, because
  that check keys on the daemon and not on the sentence.
- Anyone adding a second direct-egress path now has a test to apply —
  is this the model's traffic, or the user's own — instead of a rule
  everyone already knows one unit breaks.
