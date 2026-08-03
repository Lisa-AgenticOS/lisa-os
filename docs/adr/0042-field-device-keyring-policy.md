# ADR-0042: The field device runs a blank login keyring

- Status: **accepted** (decided 2026-08-03; awaiting one human visit
  to the iMac to apply — the keyring password is not remotely known)
- Date: 2026-08-03
- Relates: #168 (where the hang was found), the mail token fail-fast
  (cli/lisa/src/mail.rs), M7 installer (where the real policy lands)

## Context

Found chasing #168: after every reboot of the reference iMac, the
entire credential chain was silently dead. GDM autologin starts the
session but types no password, so the GNOME keyring stays locked;
GOA's `GetAccessToken` then parks behind an unlock prompt drawn for
nobody, and everything downstream — mbsync, msmtp, the sync unit —
inherited a hang with no error anywhere. The immediate defect (hanging
instead of failing) is fixed: `lisa mail token` now reads the
collection's `Locked` property and says what to do. But the underlying
state machine remains: **an autologin machine relocks its secrets at
every boot and cannot unlock them without hands.**

Remote unlock was attempted and failed — the keyring password is not
the login password, so this needs one seated visit regardless.

## Decision

On the **field-test device** (autologin, login password `lisa`,
physically accessible): recreate the login keyring with a **blank
password**. gnome-keyring then auto-unlocks at session start, secrets
survive reboots usable, and the whole failure class disappears.

The trade is stated, not hidden: a blank keyring stores OAuth tokens
unencrypted on disk. On this machine that is honest rather than
reckless — anyone with physical access already has an autologin
session and a published password; the encrypted keyring was defending
nothing except our own uptime. Security theater that costs the mail
chain a hang per reboot is worse than no theater.

**This is a field-device policy, not a product policy.** A real
install (M7 installer: TPM-LUKS, a login password actually typed)
keeps the encrypted default, where PAM unlocks the keyring with the
password the user just typed and the problem never exists. The
installer's autologin option, if it ever ships, must surface this
exact trade instead of silently choosing either side.

## Consequences

- One human step remains: delete/recreate the Login keyring in
  Passwords & Keys on the iMac, reconnect the Google account (~2 min).
- `lisa mail status` should learn to report the locked state (the
  token path already refuses loudly).
- The nightly's GOA/Secret-Service implication checks stay as they
  are; they assert presence, and this ADR is about state.
