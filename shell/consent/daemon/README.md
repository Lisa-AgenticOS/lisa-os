# shell/consent/daemon — `/usr/bin/lisa-consentd`

Spec: issue #289, ADR-0035 §4, ADR-0030, ADR-0033, PLAN §5.10. The
component as a whole is documented one directory up
(`shell/consent/README.md`); this file is about the binary.

## What it does

It is the **peer**. It owns `dev.lisaos.Consent1`, subscribes to
`dev.lisaos.Agent1`'s `ConfirmationRequested` and `RefusalReported`, and
calls `dev.lisaos.Agent1.Confirm` with whatever a person clicked. It
draws nothing: the window is `shell/consent/lisa-consentd.js`, spawned as
a child over a pipe.

## Why it exists at all

agentd decides who the human's dialog is from two facts the transport
supplies — who owns the name, and what `/proc/<pid>/exe` says that
connection is running. The dialog used to be `gjs`, so the second fact
named an **interpreter**, and an interpreter on an allowlist authorises
every program it can run. A hostile GJS script that forks and execs `gjs`
gets a fresh pid and a fresh connection, clears both of #289's checks,
and is then indistinguishable from the real dialog.

A launcher that `exec`s the GJS surface would not help — after `execve`
the exe is `gjs` again. **The process that owns the bus name has to be
the binary.**

## How it works

```
zbus session connection
  ├─ serve_at /dev/lisaos/Consent1   Ping() -> s,  PendingCount() -> u
  ├─ MatchRule: sender=dev.lisaos.Agent1, path=/dev/lisaos/Agent1
  └─ RequestName dev.lisaos.Consent1  (DoNotQueue, no AllowReplacement)
        ↑ claimed LAST — agentd emits the moment the name is owned (#244)

on a signal  ──▶ Dialogs::show ──▶ one JSON line on the child's stdin
on an answer ──▶ Confirm(call_id, approve) on THIS connection
```

Smallest real usage — what agentd does, and the only thing that ever
starts this daemon:

```
$ busctl --user call org.freedesktop.DBus /org/freedesktop/DBus \
      org.freedesktop.DBus StartServiceByName su dev.lisaos.Consent1 0
$ busctl --user call dev.lisaos.Consent1 /dev/lisaos/Consent1 \
      dev.lisaos.Consent1 PendingCount
u 1
```

Three modules, and each holds one decision:

- `protocol.rs` — the JSON-line channel. Two message kinds in, one answer
  out, and `confirm_for(Dismiss) == None` so a refusal report can never
  become a `Confirm`.
- `renderer.rs` — where the dialog file comes from (`/usr`, never the app
  channel) and what the child is not allowed to inherit
  (`DBUS_SESSION_BUS_ADDRESS`), plus the one bus it *is* handed
  (`AT_SPI_BUS_ADDRESS`, so a screen reader can read the dialog).
- `main.rs` — the connection, the name, and the loop.

## How to extend it

- **New dialog kinds** are a new `ToRenderer` variant and a new `member`
  arm in `to_renderer`. Anything not in that match draws nothing, which
  is the intended default.
- **Do not add a D-Bus method that approves.** `Ping` and `PendingCount`
  are the whole interface on purpose: a peer that could ask this daemon
  to say yes has laundered its own request through the one connection
  agentd trusts.
- **Do not add an environment variable that names the dialog file.** The
  dev override is compiled out of release builds (`debug_assertions`),
  and that is the only reason it is acceptable.

## Limits

- **It does not close #289 by itself.** `CONSENT_SURFACE_PROGRAMS` still
  lists `/usr/bin/gjs` beside `/usr/bin/lisa-consentd`, and the list is a
  disjunction. See `shell/consent/README.md` and
  `tests/injection-suite/tests/consent_program.rs`.
- **The renderer is trusted once spawned.** It can answer any dialog the
  daemon opened. What changed is *which* GJS process gets that trust: one
  the daemon started from a root-owned path, rather than any `gjs` on the
  machine that reached `RequestName` first.
- **No integration test.** The protocol, the environment discipline and
  the signal mapping have unit tests; there is no test that stands up a
  bus, activates the daemon and drives a click. The device evidence is in
  `shell/consent/README.md`.
- **A crashed renderer drops its open dialogs.** The parked calls stay
  parked in agentd and expire, which is the safe direction, but the
  person sees the windows vanish with no explanation.
