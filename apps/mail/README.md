# apps/mail — Mail

Spec: `docs/PLAN.md` §5.3 (mail as a context source), §5.8 (apps are
proof of the SDK). Milestone: M6. Read those before changing this
(CLAUDE.md rule 1).

Three panes — folders, a grouped message list, a reading pane. The
layout is the one every mail client has converged on; the grouped middle
pane is the part worth taking seriously.

## What it is for

Not that the world needs another mail client. `mail` is the context
source §5.3 cares most about and the one nothing has ever fed: the ACL
has had a `mail` provenance since M3 with no source to put anything in
it, and the prompt-injection machinery escalates on mail provenance with
no mail to escalate on. This app is that source.

So the Agent Bus tools are half the point:

| tool | tier | returns |
|---|---|---|
| `search_mail` | read | summaries — subject, sender, date, group, preview |
| `read_message` | read | one message in full, by the id `search_mail` gave |

**Everything they emit is tagged `mail` provenance**, on the JSON-RPC
envelope *and* inside the payload — agentd unwraps `content[0].text` and
discards the envelope, which is how the browser's tag was lost on its
first on-device run. The tag is a constant applied on the way out, and
the spread puts it *after* the handler's fields, so a message whose body
contains `{"provenance":"user"}` cannot relabel itself.

That tag is what makes "read my mail, then do something privileged" ask
first: agentd escalates the confirmation tier of any call whose chain
includes untrusted provenance (§5.10, Appendix C).

`search_mail` returns summaries and never bodies. A search that dumped
every match into the model's context would spend the window on the first
query and hand over far more than was asked for.

## Sync is somebody else's job

Mail reads a **Maildir**. It does not speak IMAP and will not: an OS
whose defining constraint is egress control should not grow its own
network mail client when `mbsync`/`isync`, `offlineimap` and `notmuch`
are mature and already write the format.

```
$HOME/Mail/            # or $LISA_MAILDIR
  INBOX/{cur,new,tmp}
  Sent/…  Archive/…
```

The consequence is honest: Mail shows what has been synced, and says so
when there is nothing rather than pretending to be offline.

`isync` is in the image, because "somebody else's job" only works if
somebody else is on the disk — and an immutable root cannot install one
later. `lisa mail setup` writes the config that joins it to the account
GNOME Online Accounts already holds:

```
lisa mail status     # which layer is blocking, if any
lisa mail setup      # write ~/.config/lisa/mbsyncrc, enable the timer
lisa mail sync       # one pass now
```

Four things have to be present, and `status` names the first one that is
not: **mbsync**, the **XOAUTH2 SASL mechanism**
(`os/packages/cyrus-sasl-xoauth2` — Cyrus SASL ships none), a
**credential store** (#154), and a **connected account**. Any of them
missing and mail does not arrive; only the first one missing is worth
telling you about.

An app password works instead of all of that except mbsync:
`lisa mail setup --app-password ~/.mail-password`. Some providers offer
nothing else, and it needs neither keyring nor SASL plugin.

**Nothing in the generated config can destroy mail on the server** —
every channel carries `Expunge None` and `Remove None`.

## How it works

| file | what |
|---|---|
| `lib/rfc822.js` | headers, encoded words, addresses, readable body. Pure, total — malformed input yields something empty, never an exception |
| `lib/maildir.js` | filename flags, listing, ids, path containment, previews |
| `lib/smart.js` | which pile a message goes in |
| `lib/actions.js` | what the buttons do, as arithmetic on filenames |
| `lib/mcp-protocol.js` | JSON-RPC + the provenance tag. Pure |
| `lib/mcp.js` | the socket |
| `lib/settings.js` | what the settings page is allowed to say. Pure |
| `lisa-mail.js` | the window; thin over the above |

`just shell-test` runs the pure half — 46 cases, on any dev host.

## Settings

A diagnostic before it is a preference sheet. Connect Google in Settings,
open Mail, see nothing: every layer is behaving as designed and not one
of them can say so. GOA holds an account it cannot get a token for, the
Maildir is empty because nothing fills it, and an empty folder is the
correct thing to draw. The failure lives in the gaps, which is where
nothing is looking.

So the page reports facts and names the gap:

- **Maildir** — the folder, where the path came from (`LISA_MAILDIR`, the
  saved setting, or `~/Mail`), and what is in it right now. When the
  environment set it, the row says so and stops being editable: an env
  var a stored preference can silently override is a debugging trap.
- **Syncing** — the first blocking answer, in the order the layers block
  each other: no mbsync → no keyring (#154) → no account → nothing
  bridges them (#155). Order is the point. Telling somebody their account
  is fine while the machine has no syncer sends them to debug the wrong
  layer.
- **Accounts** — what GOA reports, including an account with Mail
  switched off, which from inside this app looks identical to no account
  and is a different problem. Adding one opens GNOME Settings rather than
  reimplementing an OAuth flow.

Config is `~/.config/lisa/mail.json`, plain JSON, and a malformed one
costs you the preferences and not the app — a mail client that will not
start is the hardest kind of thing to fix from inside a desktop session.
No GSettings schema, because a schema has to be compiled into the
session's schema directory to be readable at all, and this app is meant
to be runnable from a checkout.

## The buttons

A Maildir action is a rename, so `lib/actions.js` computes the new path
and the window performs it. Three things that are easy to get wrong and
are therefore tested rather than assumed:

- **Flags are written in ASCII order.** Clients disagree about plenty;
  they agree about this, and unsorted flags make other clients mis-sort
  the same Maildir.
- **Acting on a message in `new/` moves it to `cur/`.** That is what the
  two directories mean. Skipping it leaves read mail looking unread to
  everything else sharing the Maildir.
- **A move lands in `cur/`, never `new/`.** Filing a message into another
  folder's `new/` makes it unread again and, on a synced Maildir,
  notifies about a message you just filed.

**Trash is a move to the Trash folder, and the button says "Move to
Trash".** Maildir's `T` flag means "deleted", but what actually destroys
a message is a separate expunge — a bin icon that destroys immediately
is not a button, it is a trap.

**Reply and Forward are shown, disabled, and say why on hover.** Sending
needs SMTP: credentials and egress, a different class of thing from
renaming a file, and one that belongs behind the same scrutiny as any
other egress here. A mail app with no reply button reads as unfinished;
one whose reply button does nothing reads as broken. Saying why is
neither.

These are **UI actions only** — not Agent Bus tools. They are write-tier
by nature, and write-tier tools should wait for the consent surface to
be split from the model host (#145).

**Opening a folder opens its first message**, and the buttons live in the
reading pane's header. That coupling is the whole reason it works this
way: a pane with nothing open is an app that appears to have no actions
at all, which is exactly how the toolbar was first reported missing.
Opening does *not* mark the message read — `S` is set by the button, by a
person deciding they have read it.

**A button whose icon does not resolve falls back to its label**, and
says so on stderr. Adwaita has been retiring legacy icon names for
several releases and `box-symbolic` — the obvious name for Archive — was
never in it at all; mail clients that show an archive box ship their own
copy. An unresolved name gives you an empty button, which is
indistinguishable from no button.

## Extending it

A new smart group is a branch in `classify` plus a name in `GROUPS`, and
a test. Order matters there and is documented in the function: `Pinned`
outranks everything because it is the user's own decision, `Seen` next
because a read message leaves the working set whatever it is, and
`People` is the default — a misfiled message from a person is the
expensive error, a misfiled newsletter is not.

A new tool is a handler in `lisa-mail.js`, a line in the `McpServer`
table, and an entry in `app.lisaos.Mail.json`. Keep `maxLength` bounds at
or under 256: a larger one breaks grammar compilation for **every**
offered tool, not just this app's (issue #147).

## Settings

A real preferences page, not only the diagnostic it started as.

- **Show images in messages** — default **on** (project owner's
  decision, 2026-08-02). The trade is stated on the page rather than
  buried: a remote image is how a sender learns you opened a message,
  when, and roughly from where. Off restores the per-message banner.
  Toggling it re-renders the message on screen, not just the next one.
- Maildir location, on-disk counts, and the sync diagnostic that
  explains *why there is no mail* — the reason this page exists at all.

## Limits

- **No sending.** Reply, forward and compose need SMTP — credentials and
  egress — and the buttons are shown disabled rather than hidden.
- **No permanent deletion.** Trash is a move; nothing here expunges.
- **Actions are the window's, not the agent's.** `search_mail` and
  `read_message` are the only Agent Bus tools; pinning, archiving and
  flag changes are write-tier and wait on the consent surface split
  (#145).
- **No threading.** Messages are listed individually. `References` and
  `In-Reply-To` are parsed and unused.
- **No attachments.** MIME parts other than the readable body are
  ignored — not listed, not saved.
- **No search index.** `search_mail` scans folders; fine for thousands
  of messages, not for hundreds of thousands. The contextd path (§5.3,
  proper indexing with embeddings) is the answer when it matters.
- **Not indexed into contextd yet**, so mail does not appear in
  `[my stuff]` retrieval — only through these tools. That is the next
  step and it needs an ingestion API contextd does not have.
