# lisa-contextd — context fabric

Spec: docs/PLAN.md §5.3. Milestone: M3.

System-wide personal context index (files, mail, calendar, chat, screen —
each individually consented) plus per-app durable memory. SQLite + FTS5
(+ vectors), every retrieval ledgered with provenance tags and per-chunk
scope ACLs.

## Status: M3 core + hybrid + scoped-ACL (landed)

Implemented and unit-tested (macOS + Linux, no daemon required):

- **File ingestion** (`index.rs`) — walk → text-filter → ~1 KiB
  paragraph chunks → FTS5, incremental (mtime + blake3 hash skip
  unchanged, atomic reindex of changed). Every chunk carries a
  provenance tag.
- **Lexical retrieval** (`index.rs::search`) — FTS5 bm25, best-first,
  with provenance + snippet.
- **Hybrid retrieval** (`embed.rs`) — per-chunk embeddings + BM25×cosine
  blend over FTS-prefiltered candidates (sqlite-vec at scale is the later
  optimization). `embed_pending`, `search_hybrid`.
- **Per-app memory** (`memory.rs`) — namespace-isolated key/value with
  zero-residual wipe (an app never reads another's namespace).
- **Scoped-ACL retrieval** (`acl.rs`) — maps a granted portal scope to
  the provenance it may read and filters *at the query*, so a
  disallowed-provenance chunk can't leak through ranking even when it
  ranks best. Deny-by-default on empty/unknown scopes. `search_scoped`;
  ACL-leak + fuzz tests assert **0 cross-scope leaks** (§5.3 acceptance).
  `add_document` ingests non-file (mail/screen/web) provenance.

CLI: `lisa context index [--embed]`, `lisa context search [--hybrid]
[--scope <scope>]` (scoped searches ledger as `context.search.scoped`).

## D-Bus surface (landed)

The `lisa-contextd` binary (`src/main.rs`) owns **`dev.lisaos.Context1`**
on the session bus, object `/dev/lisaos/Context1` (`src/dbus.rs`):

- `Search(query s, options a{sv}) → s` — options `limit` (u, default
  3), `hybrid` (b), `scopes` (as; present ⇒ the ACL-scoped path,
  deny-by-default). Returns a JSON array of
  `{source, provenance, snippet, score}`. The
  `context.search[.hybrid|.scoped]` ledger entry is appended **before**
  the store is queried; append failure refuses the search (dataflow
  rule 4).
- `MemoryGet/MemorySet/MemoryList/MemoryWipe(app …)` — the library's
  namespace-isolated per-app memory, verbatim (`MemoryList` returns a
  JSON object; a missing key errors).
- `Ping() → s`.

The daemon exits when its bus connection dies (watchdog, same as
inferenced) so systemd re-registers the name. It never gets network
access (CLAUDE.md rule 5); the packaged user unit
(`os/packages/lisa/lisa-contextd-user.service`) denies the address
families outright, and `dev.lisaos.Context1.service` D-Bus-activates it
on first call. First consumer: the assistant overlay backend's
[my stuff] retrieval (`shell/overlay-extension/backend/lisa-overlayd.js`,
with a CLI shell-out fallback). Tested over zbus p2p in `tests/dbus.rs`.

## Who is asking

Both halves of `dev.lisaos.Context1` used to take the caller's word.

**Memory namespaces** (#101) were a method argument with no check, so any
peer on the session bus could read another app's durable memory — the
assistant stores whole session transcripts there — or wipe it. The app id
now comes from the caller (`lisa_peer::app`); an `app` argument may only
*match* it, and an empty one means "mine". Naming somebody else's
namespace works only for an allowlisted program, which is how
`lisa memory --app X` stays possible.

**Search** (#100) picked its path from a caller-supplied option: `scopes`
present meant the ACL, absent meant *every provenance, unfiltered*. The
ACL was a request, not a boundary — and the one shipping consumer, the
overlay's [my stuff], omitted the key. Now absence means the same as an
empty list: nothing. An unscoped read is available only to the user's own
tooling, because `lisa context search` is a person looking at their own
index and a guardrail between a person and their own machine is the wrong
guardrail (ADR-0030).

Retrieval entries name the app, its identity kind, and the effective
scopes — §5.3 asks for that and the old `app_id: "host"` could not
provide it.

### Scopes and provenance

`screen.once` grants **nothing** here (#112). It is the portal's
per-invocation scope — one "share this window" — and mapping it to the
whole `screen` provenance turned that single consent into a durable read
of every capture ever indexed, over the most sensitive class in the
store, in a system that explicitly refuses to build a Recall (§5.7.4). A
durable historical read would need its own scope name so a consent dialog
could say so; none is invented until something needs one.

Writes are validated against the known provenance set. An unknown tag
used to be accepted and then readable by no scope at all, which looks
exactly like an empty index rather than a typo.

`index_dir` refuses to touch a source already indexed under a non-`file`
provenance (#104) and names what it skipped. It used to key on the path
alone and re-insert with a hardcoded `'file'`, so a plugin whose source
id is a path inside the walked tree — an exported message, a web or
screen cache file — was silently relabelled and became
`documents.read`-readable. That was content-dependent too: an unchanged
hash took the skip branch and kept the old label, so the same corpus
classified differently depending on whether the bytes matched.

## Left for M3 completion

Live sources + watchers (file/mail/calendar ingestion daemons),
sqlite-vec at scale, encryption-at-rest (keyring), portal-mediated
caller identity on the D-Bus surface (M2 attaches per-app identity;
until then callers pass the app id and the ledger records `host`), and
the Settings › Intelligence panel. Provenance is
load-bearing (CLAUDE.md rule 6): ingestion never lets an untrusted caller
forge a provenance tag — real mail/screen chunks arrive via
portal-mediated sources, not a raw CLI flag.

Prior art: [cognee evaluation](../../docs/notes/cognee-evaluation.md) —
knowledge-graph memory platform; not the substrate (Python, multi-engine),
but a flagship M5 MCP tenant and a design reference for the M3
entity-graph question.
