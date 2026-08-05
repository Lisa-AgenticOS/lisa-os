# tests/e2e — full-image tests

Spec: docs/PLAN.md §11. Milestone: M0→M1.

QEMU+swtpm driving the real image: boot → grant → inference → ledger assertions via busctl/DB. Includes the A/B update→rollback demonstration (M0 acceptance).

Status: **not started** — scaffold placeholder. Read the spec section (and CLAUDE.md rules) before writing code here.

## What does run today

Three scripts here are real and run in CI, ahead of the image suite above.

**`egress-test.sh`** — CLAUDE.md rule 5, executed. For every unit that
`os/repo-tools/check-egress-units.py` classifies no-egress, it lifts that
unit's `[Service]` sandbox out of the **shipped file**, applies it with
`systemd-run -p`, and tries to reach the internet from inside it. Six units
today (lisa-inferenced ×2, contextd, agentd, harnessd, notes); before #275
it covered lisa-inferenced alone, under a sandbox the script had re-typed
by hand. Bracketed by two positive controls — unsandboxed egress must
work, and egress under `lisa-remoted`'s own sandbox must work — plus a
per-unit `curl --version` that must succeed before that unit's "blocked"
counts for anything.

    bash tests/e2e/egress-test.sh [path/to/lisa-inferenced]

Linux with systemd and sudo. Without the binary argument the daemon
liveness check is skipped and everything else still runs.

Its limits, because they are not obvious:

- Only lisa-inferenced additionally gets its *daemon* run under the sandbox
  and probed for liveness — it is the only one of the six answering over
  HTTP; the rest are D-Bus/unix-socket daemons needing a session bus that a
  system-scope `systemd-run` has not got. Their sandbox is proven, their
  liveness under it is not.
- It proves the *property*, not the spelling. Delete `IPAddressDeny` from a
  unit that also carries `RestrictAddressFamilies=AF_UNIX` and this stays
  green — correctly, since egress is still blocked. Noticing the lost layer
  is `check-egress-units.py`'s job. Run both; CI runs both.
- `xdg-desktop-portal-lisa.service` is classified no-egress but exempt: it
  ships no egress sandbox at all today, and a process under it does reach
  the internet (verified 2026-08-05). The exemption lives in
  `check-egress-units.py`, which fails the day the unit is hardened — so
  the exemption deletes itself rather than accumulating.

**`layer-test.sh`** — the Track L install/uninstall e2e (M0 acceptance),
run in an Arch systemd container.

**`openai_client_test.py`** — the OpenAI-compatible surface, driven by the
real `openai` client against a running `lisa-inferenced`.
