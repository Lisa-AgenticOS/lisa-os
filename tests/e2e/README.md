# tests/e2e — full-image tests

Spec: docs/PLAN.md §11. Milestone: M0→M1.

QEMU+swtpm driving the real image: boot → grant → inference → ledger assertions via busctl/DB. Includes the A/B update→rollback demonstration (M0 acceptance).

Status: **not started** — scaffold placeholder. Read the spec section (and CLAUDE.md rules) before writing code here.

## What does run today

Three scripts here are real and run in CI, ahead of the image suite above.

**`egress-test.sh`** — CLAUDE.md rule 5, executed. For every unit that
`os/repo-tools/check-egress-units.py` classifies no-egress, it lifts that
unit's `[Service]` sandbox out of the **shipped file and its shipped
drop-ins**, applies it with `systemd-run -p`, and tries to reach the
internet from inside it. Before #275 it covered `lisa-inferenced` alone,
under a sandbox the script had re-typed by hand. Bracketed by two positive
controls — unsandboxed egress must work, and egress under `lisa-remoted`'s
own sandbox must work — plus a per-unit `curl --version` that must succeed
before that unit's "blocked" counts for anything.

    bash tests/e2e/egress-test.sh [path/to/lisa-inferenced]

Linux with systemd and sudo. Without the binary argument the daemon
liveness check is skipped and everything else still runs.

The unit list is not written down anywhere — not here, not in the script.
Ask the classifier:

    python3 os/repo-tools/check-egress-units.py --explain

An earlier version of this paragraph named six units while the gate
classified seven (#295). A hand-maintained copy of a derived list is the
defect the derived list was introduced to remove, so there is no copy.

Its limits, because they are not obvious:

- Only lisa-inferenced additionally gets its *daemon* run under the sandbox
  and probed for liveness — it is the only no-egress daemon answering over
  HTTP; the rest are D-Bus/unix-socket daemons needing a session bus that a
  system-scope `systemd-run` has not got. Their sandbox is proven, their
  liveness under it is not.
- It proves the *property*, not the spelling. Delete `IPAddressDeny` from a
  unit that also carries `RestrictAddressFamilies=AF_UNIX` and this stays
  green — correctly, since egress is still blocked. Noticing the lost layer
  is `check-egress-units.py`'s job. Run both; CI runs both.
- It runs in **system scope**, and two of the three directives it exercises
  do not mean the same thing there. `IPAddressDeny` / `IPAddressAllow` are
  a cgroup BPF program that `systemd --user` cannot load, so for a per-user
  unit they are live here and inert on the device (#288). That makes this
  harness stricter than reality, never weaker — but a "blocked" it reports
  for a user unit may be a block the machine does not have.
  `RestrictAddressFamilies` is a seccomp filter and behaves identically in
  both scopes; it is the only one of the three that confines a user unit,
  which is why `check-egress-units.py` asserts it by name.
- What it does NOT assert, contrary to what `os/packages/README.md` still
  says: it never checks a named directive. `DynamicUser`,
  `IPAddressAllow=localhost` and the filesystem/kernel lockdown are applied
  by being lifted wholesale out of the unit, not verified. Only
  `IPAddressDeny=any` is checked by name, and only in the static gate.

**`layer-test.sh`** — the Track L install/uninstall e2e (M0 acceptance),
run in an Arch systemd container.

**`openai_client_test.py`** — the OpenAI-compatible surface, driven by the
real `openai` client against a running `lisa-inferenced`.
