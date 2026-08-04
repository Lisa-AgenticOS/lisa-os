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
| `read_message` | read | one message in full, by the id `search_mail` gave, plus a list of what is attached (name, type, size) |

**Neither tool serves Spam, Junk or Trash** (#237). That is contextd's
decision from #185 — Spam is "a corpus that exists to be hostile" and
Trash is what you already chose to forget — and the Agent Bus was
reaching the same corpus through a second door with no index in between.
The list is the one `lisa mail index` uses
(`cli/lisa/src/mail.rs`, `UNINDEXED_FOLDERS`), matched on any path
component, case-insensitively. **The window still shows them**: a person
opening their own Spam folder is their business, which is ADR-0029's
second test — a guardrail sits between the model and the machine, never
between somebody and their own mail.

The Write-tier tools are not restricted this way: they return no message
content, so they are not a route into the corpus, and an agent has no id
to act on there anyway — `search_mail` hands none out.

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

### Two layouts, and both of them at once

`lisa mail setup` writes a flat tree for one account and a per-account
tree for several:

```
~/Mail/INBOX/…                          # flat
~/Mail/you_at_example.com/INBOX/…       # per account
```

**A root can hold both, and the reference device does** (#222). Account
discovery used to stop at the first flat folder it saw, so a machine
with an old flat tree *and* two synced accounts reported one account
called "Mail": both real accounts were drawn in the sidebar as empty
folders, 24,456 messages were unreachable, `search_mail` answered out of
the leftover tree, and Sent and Drafts were written into it.

Now a directory is a folder only if it holds `cur/` or `new/`, an
account is a directory of folders, and **both kinds are reported**. The
named accounts come first, because the store opens on the first one and
live mail is what a person means; folders sitting loose in the root come
last, as an account named `Mail`.

**Nothing is moved and nothing is deleted.** A tree that looks like a
leftover is still somebody's mail, and an app that tidies it away has
made an irreversible decision on their behalf. Mail shows it, names it,
and leaves it where it is.

The agent tools see **the selected account** — `search_mail` walks the
folders of whichever account the window is on, because a message id is
`folder/message` and has nowhere to put an account. Two accounts with a
folder of the same name are two different folders; asking across both
would need an id shape that says which.

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
| `lib/rfc822.js` | headers, encoded words, addresses, readable body, the MIME part walk and transfer decoding. Pure, total — malformed input yields something empty, never an exception |
| `lib/message.js` | one message file → the object the reading pane, the tools and Reply all share. Pure |
| `lib/attachments.js` | what is attached, what its bytes are, and what it is safe to call the file. Pure |
| `lib/maildir.js` | filename flags, listing, ids, path containment, previews |
| `lib/smart.js` | which pile a message goes in |
| `lib/actions.js` | what the buttons do, as arithmetic on filenames |
| `lib/mcp-protocol.js` | JSON-RPC + the provenance tag. Pure |
| `lib/mcp.js` | the socket |
| `lib/settings.js` | what the settings page is allowed to say. Pure |
| `lisa-mail.js` | the window; thin over the above |

`just shell-test` runs the pure half — 135 cases, on any dev host.

### A message is bytes until its charset says otherwise

Every message file is read one character per byte (`messageText`), and
the parser decodes each part once it knows what the part declares. The
app used to read mail through `TextDecoder('utf-8')`, which replaces
every byte sequence that is not valid UTF-8 with U+FFFD *before the
message has said what its charset is* — so the Latin-1 branch of the
decoder could never run and `charset=ISO-8859-1; 8bit` mail arrived as
`P<FFFD>rsh<FFFD>ndetje` (#232). On the reference device that was 402
message bodies carrying replacement characters; it is now 140, and what
is left are messages that declare **no charset at all** and contain
Windows-1252 bytes (101 of them), or declare `utf-8` and are not
(39) — where a replacement character is the honest answer rather than a
guess.

The consequence for anything calling `lib/rfc822.js`: **its input is a
byte string, not a decoded one.** Fixtures in the suites build their
non-ASCII cases out of bytes for that reason, and because a fixture that
is already correctly decoded cannot fail the way real mail does.

## Attachments

A message with a PDF invoice used to look exactly like a message without
one. The parser walked the MIME tree, decoded every leaf as text, kept
the `text/plain` and `text/html` parts and dropped the rest — so the
invoice that said *"Dokumenti është bashkëngjitur si PDF"* was a message
telling you to open a file the reader had thrown away (#211, #169).

Now every non-body part is listed under the sender, with its name, its
declared type and its size, and three things you can do with it:

| action | what happens |
|---|---|
| **Save As…** | a `Gtk.FileDialog` — **you** choose the directory; the sender's filename is only the suggested name |
| **Open** (Enter, or the button) | the file is written to a private temp directory and handed to **Preview**, by desktop id |
| **Space** | the same quick look Files has — the transient peek, toggled closed by pressing Space again |

Space goes through `org.gnome.NautilusPreviewer` — the *versionless* bus
name; only the interface carries the `2` — with
`CloseIfAlreadyVisible: true`, which is what makes it a toggle. The
constants come from `apps/preview/lib/previewer-protocol.js` rather than
being spelled again here, because that name has been gotten wrong once
already and the wrong one produces no bus traffic at all.

**Space is owned by the attachment list and nothing else.** The key
controller is attached to that list, so it is only on the event's path
while focus is inside it: Space in the message list, in the search entry
or in any text field behaves exactly as it did. It runs in the capture
phase (GtkListBox has its own Space binding) with one explicit
exception — a focused Save or Open button keeps Space, because that is
how a button is pressed from the keyboard.

**Open and Space are different actions.** Open launches Preview proper,
which stays in the dock; Space is the transient peek, which does not.

### The filename is the sender's, and is treated that way

`safeFilename` (`lib/attachments.js`) takes the basename, drops NUL,
control characters and bidi overrides, refuses a name that is only dots,
drops a leading dot, and caps the length while keeping the extension.
`../../.ssh/authorized_keys` becomes `authorized_keys`. Nothing is ever
written to a sender-chosen path: the target is either a directory the
person picked in a file dialog or a directory this app just made with
`make_tmp`, and the temp path is checked to be inside it afterwards.
The declared MIME type is a claim too, so it is reduced to the token
characters a MIME type may contain before it is put in a label.

Extraction is bounded: at most 64 MB per part, and a message file over
128 MB is not read at all. A malformed part decodes to whatever its
valid characters said rather than throwing, and the walk stops at six
levels and two hundred parts, because forty nested multiparts are a
cheap thing to send.

**A part past the bound is refused, not truncated** (#238). The size in
the row is computed by arithmetic and the bytes are decoded with a cap,
and nothing compared the two: a 68 MB attachment showed 71,303,169 bytes
and Save As wrote 67,108,864 of them, silently. Three quarters of
somebody's file, written under the name of the whole thing, is the worst
failure this app could have — it opens far enough to look like their
problem. Now `attachmentBytes` returns the whole part or nothing, a save
the person asked for is allowed the whole message file's worth (a
decoded part cannot exceed the file it came from), and anything past
even that says so on the banner.

### The bytes are for the person, not for the model

`read_message` reports **what** is attached — filename, type, size — and
never the contents. An agent asked "is the invoice attached?" can answer
it and point at Preview, which is where a document gets read; a
summariser that could be handed a PDF is one that can be handed anything
a sender likes.

**That was a promise this app broke for as long as it has had an Agent
Bus** (#221). Choosing the body fell back to "the first part with
anything in it", with no check that the part was text — so ordinary
"here is the file" mail, where the text parts exist and are *empty*,
resolved to the PDF, the JPEG or the .docx. `read_message` returned a
3,145,615-character body starting `PK\x03\x04`; the list's preview
column read `%PDF-1.4 %Ǭ… /FlateDecode`. Measured across the reference
device's 34,368 messages: **168 bodies, 98 MB of decoded binary, now
zero.**

A message whose only content is a document has an **empty body and a
listed attachment**, which is the truth. The reading pane says so in
words rather than showing a blank pane, because a blank pane is how
#210 was reported.

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

**The socket is given back on every exit, not only on a window close.**
mcp-bus defers socket activation and reads presence as availability, so
a socket left behind by a killed process is a tool that advertises
itself and answers `ECONNREFUSED` (#219). `close-request`, GApplication
`shutdown`, and SIGHUP/SIGINT/SIGTERM all release it, once.

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

- **Sending: reply and forward work (#168); a standalone Compose entry
  point does not yet.** `lib/compose.js` builds the message (Re: that
  does not stack, a References chain so threading survives in the
  recipient's client, base64 body so a long line cannot be rejected),
  `lib/send.js` decides what an msmtp run meant, and the window keeps
  the text on screen when a send fails. Draft-first: the message is
  written to Drafts *before* msmtp runs and removed only on success, so
  a crash or a refusal leaves what you wrote on disk. Needs
  `lisa mail setup` to have written `~/.config/lisa/msmtprc`; without
  it Reply and Forward are shown disabled and say so.
  **Not yet exercised against a real account** — see the commit for what
  was and was not verified.
- **No permanent deletion.** Trash is a move; nothing here expunges.
- ~~Actions are the window's, not the agent's.~~ **Done (#167):**
  `flag_message`, `mark_read`, `archive_message`, `trash_message` and
  `move_message` are Write tier, resolved by `lib/agent-actions.js` into
  the same `flagChange`/`moveTo` plans the toolbar uses — one
  implementation, two callers. An action that would change nothing
  returns `changed: false` with a reason rather than a cheerful ok.
- **No threading in the list.** Messages are shown individually. What a
  reply *sends* does thread: `Message-ID` and `References` are read off
  the message and a reply carries `In-Reply-To` and the extended chain
  (#223). It did not until now — nothing in this app called
  `headers.get('message-id')`, `replyFields` read a field no producer
  set, and `?? ''` swallowed the absence, so every reply this app ever
  composed landed in the recipient's client as a new conversation. The
  test that should have caught it supplied both fields by hand.
- ~~No attachments.~~ **Done (#211, #169):** listed, saved, opened in
  Preview, and quick-looked with Space — see *Attachments* above. Three
  things it still does not do:
  - **`cid:` inline images do not render in the HTML body.** The
    reading pane resolves nothing from the message's own parts, so a
    newsletter's inline logo is a broken image, exactly as it was
    before. Those parts are parsed and kept out of the attachment list
    (they are body imagery, not files); connecting them to the WebView
    needs a custom URI scheme handler and is its own change (#169).
  - **Nothing is sent with an attachment.** `lib/compose.js` builds a
    single-part message; Reply and Forward carry text only.
  - **Attachment contents are not indexed into contextd** (#170 indexed
    message bodies; the documents inside them are a separate issue).
- **No search index.** `search_mail` scans folders; fine for thousands
  of messages, not for hundreds of thousands. The contextd path (§5.3,
  proper indexing with embeddings) is the answer when it matters.
- **Indexed into contextd** (#170): `lisa mail sync` indexes new
  messages into the context store under `mail` provenance (and
  `lisa mail index` backfills), so retrieval can answer "the email
  about the parking permit" — semantically, with the real embedder.
  The ACL holds: an app with only `documents.read` never sees a mail
  chunk. No writable D-Bus ingestion API exists, deliberately — the
  CLI indexes in-process, so there is no surface for another app to
  poison. Attachment *contents* are not indexed (their own issue).
