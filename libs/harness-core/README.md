# harness-core

The assistant pillars (ADR-0013 phase 2), ported from the *design* of
[flakerimi/harness](https://github.com/flakerimi/harness) onto Lisa's
substrate. The Go harness is the template; the engine is Rust, and the
crate is a plain sync library — no HTTP, no D-Bus, no daemons. Callers
send a `Turn`'s request body to an OpenAI-compatible endpoint themselves
and route actions through the Agent Bus, so every pillar inherits tiers,
provenance escalation, undo, and the Ledger for free.

## Pillar status

| Pillar (harness template) | Status | Here |
|---|---|---|
| Sessions | done | `SessionStore` over the `KvStore` seam: one JSON value per session (`session/<id>`) plus an index (`sessions`) in a `dev.lisaos.Context1` app-memory namespace — the substrate the Assistant already persists through. create / list / load / append / prune; turn wire shape (`{role, text, model}`) matches the Assistant's stored conversation payload, so it can adopt multi-conversation without new daemon surface. |
| Memory | done | `Memory`: per-scope durable notes in a caller-owned SQLite file — `remember` (with tags), `recall` (FTS5, LIKE fallback; recall reinforces), `digest` (the bounded prompt injection, reinforcement + recency ranked). SQLite rather than KV because search is the point (same FTS5 approach as contextd). |
| Skills | done | `Skill::load_dir` (SKILL.md frontmatter, lazy bodies, skip reasons), `catalog_line` progressive disclosure, optional `tools:` allowlist, and `LoadReport::resolve` — deterministic token routing (registry.rs / agent.js weights; a genuine name-token hit required). No model in the loop at this layer. |
| Soul (identities) | partial | The persona is a caller-supplied string on `Turn`; no profile files, tiers, or delegation yet. |
| Turn composition | done (text turns) | `Turn`: persona + memory digest + skill catalog + windowed history + input → chat-completions body; optional guided generation via liblisa GBNF. **Tool-call turns are the flagged wave-3 gap** (Kimi.md backlog). |
| Crons | not started | Deliberately: the wave-3 backlog names tool-call turns as this crate's next item, not schedules. |
| Hands (workspace + file tools) | not started | forge-harness's Jail is the intended boundary when this lands. |
| Background tasks | not started | — |
| Self-improvement | not started | Needs sessions + skills, which now exist. |

## Storage design

`store::KvStore` mirrors the Context1 app-memory verbs (`MemoryGet`/
`MemorySet`) within one app's namespace; on Lisa the caller implements
it over a `Context1` D-Bus proxy, tests use `store::MemKv`. Context1 has
no per-key delete, so `remove`'s default tombstones with the empty
string and readers treat empty as absent (a Context1 `MemoryDelete`
would make pruning physical — until then `MemoryWipe` remains the only
full erase). Stored values are never trusted: a corrupt session index
degrades (junk dropped), a corrupt session record is a loud
`Error::Corrupt`.

## Use

See the crate-level doc example (`cargo doc -p harness-core`): open a
`Memory`, create a session in a `SessionStore`, compose a `Turn`, POST
its `request_body()`, append the reply.
