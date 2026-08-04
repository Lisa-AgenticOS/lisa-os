# apps/mail-patches — moot, awaiting a decision

Spec: docs/PLAN.md §5.8. Decision: **ADR-0048**.

## Status: superseded, not deleted

This directory was scaffolded on 2026-07-20 to hold a patch set against
an upstream mail client. **No patch was ever written** — it has only ever
contained this README.

It is moot, because the question it was scaffolded to answer was answered
by writing the app: **`apps/mail` exists** — a GJS/GTK4 mail client,
MCP-native, shipped and reviewed. ADR-0048 generalises that outcome:
we write the apps, we do not patch GNOME's.

The two sibling scaffolds became Lisa apps on 2026-08-04
(`apps/files-patches` → `apps/files`, `apps/photos-patches` →
`apps/photos`). This one has no equivalent, because Mail is not
not-started — it is done.

**Nothing here is deleted without the owner's explicit say-so.** The
directory stays until that decision is made. If you are that owner: the
options are to remove it (`git rm`, recoverable from history) or to keep
it as a marker of the road not taken.

Do not write patches here.
