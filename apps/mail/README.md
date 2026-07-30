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

## How it works

| file | what |
|---|---|
| `lib/rfc822.js` | headers, encoded words, addresses, readable body. Pure, total — malformed input yields something empty, never an exception |
| `lib/maildir.js` | filename flags, listing, ids, path containment, previews |
| `lib/smart.js` | which pile a message goes in |
| `lib/mcp-protocol.js` | JSON-RPC + the provenance tag. Pure |
| `lib/mcp.js` | the socket |
| `lisa-mail.js` | the window; thin over the above |

`just shell-test` runs the pure half — 23 cases, on any dev host.

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

## Limits

- **Read-only.** No reply, no compose, no flag changes, no delete. The
  toolbar in a mail client implies all of those and none of them exist
  yet; write-tier tools also need the consent surface split (#145)
  before an agent should be able to reach them.
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
