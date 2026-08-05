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
  **Whose queue** matters as much as which embedder: `embed_pending`
  drains *every* pending chunk in the store, which is right for an
  explicit `lisa context index --embed` and wrong for anything with a
  deadline. `embed_pending_provenance(embedder, "system")` embeds one
  tag's chunks and leaves the others exactly where they were — the
  boot-time knowledge sync owns 28 chunks and used to inherit 90,000
  mail chunks from a backfill, which killed it at `TimeoutStartSec` on
  every boot (#192).
  **A cold embedder is not a failure**: `RetryingEmbedder::new(inner,
  attempts, base_delay)` retries transport-level errors (connect
  refused, broken pipe, unexpected EOF) with doubling backoff and a
  budget you can compute — `max_backoff()` says how long the worst case
  sleeps, `max_duration(request_timeout)` how long it takes in total.
  Errors the server actually answered (HTTP 400, a malformed body, a
  vector-count mismatch) are returned on the first try.
  **A stalled embedder is not an infinite wait**: `InferencedEmbedder`
  sets a read *and* write timeout on its socket, so a `lisa-inferenced`
  that accepts the connection and then goes quiet — model loading, a
  wedged engine — fails with a timeout instead of blocking until
  systemd kills the caller. It bounds *silence*, not duration: the
  timeout is per read syscall, so a batch that takes minutes but keeps
  sending bytes is never cut off. Two profiles, because one number
  cannot serve both a 120s boot unit and a multi-hour backfill —
  `DEFAULT_REQUEST_TIMEOUT` (180s) for `lisa context index --embed`,
  the mail backfill and hybrid search; `BOOT_REQUEST_TIMEOUT` (30s) via
  `resolve_with_timeout` for `lisa context sync-knowledge`, whose
  `max_duration` must fit inside `TimeoutStartSec=120`. Both constants
  carry the measured numbers they are derived from.
  **Which embedder** is decided by `embed::resolve()` and always
  reported: `InferencedEmbedder` when `lisa-inferenced`'s unix socket
  answers, `HashEmbedder` otherwise. The fallback is never quiet — a
  warning in the log, a note on the CLI, and `"embedder"` in the search's
  Ledger entry — because for a year `hybrid=true` returned
  plausibly-ranked hits with no semantic model behind them and nothing
  said so (#163).
  The socket rather than `127.0.0.1:7777`: this daemon runs
  `RestrictAddressFamilies=AF_UNIX` + `IPAddressDeny=any`, so it cannot
  open an IP socket at all — loopback is still the network stack. The
  companion passes `--socket %t/lisa/inferenced.sock`.
- **Per-app memory** (`memory.rs`) — namespace-isolated key/value with
  zero-residual wipe (an app never reads another's namespace).
- **Scoped-ACL retrieval** (`acl.rs`) — maps a granted portal scope to
  the provenance it may read and filters *at the query*, so a
  disallowed-provenance chunk can't leak through ranking even when it
  ranks best. Deny-by-default on empty/unknown scopes. `search_scoped`;
  ACL-leak + fuzz tests assert **0 cross-scope leaks** (§5.3 acceptance).
  `add_document` ingests non-file (mail/screen/web) provenance.
- **Mirror pruning, previewable** (`acl.rs`) — `prune_not_in(provenance,
  keep)` removes every document of one provenance that `keep` does not
  name, with its chunks *and its vectors*; `plan_prune_not_in` returns
  the same set as a `PrunePlan { sources, chunks, vectors }` without
  removing anything. Both run the same query, so the preview is the
  deletion rather than an estimate of it. The caller that needs this is
  the mail indexer (#224): 9,094 documents and 26,117 vectors on the
  reference device belonged to a Maildir tree nothing syncs, and a
  five-figure delete decided by how a config parsed is exactly the
  operation that should be printable first. A prune that dropped the
  document rows and left the `chunk_vectors` rows would satisfy every
  query that goes through `documents` and free none of the ~80 MB, so
  the tests count that table directly.

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
