# Cognee evaluation — prior art for the memory layer

- **Date:** 2026-07-21 · evaluated against PLAN §5.3 (`lisa-contextd`) and §5.4 (Agent Bus)
- **What it is:** [cognee](https://github.com/topoteretes/cognee) — Apache-2.0, Python.
  Open-source "AI memory platform for agents": ingests documents/chat/data,
  builds a **knowledge graph + vector embeddings**, exposes
  `remember / recall / forget / improve`. Runs fully embedded-local
  (SQLite + LanceDB + Kuzu) or on a single Postgres; pluggable LLM
  providers (OpenAI-compatible endpoint works). Ships an official **MCP
  server** (`cognee/cognee-mcp`; stdio/SSE/HTTP).

## Verdict

**Not the substrate, but a flagship tenant — and a design reference.**

1. **Not a `lisa-contextd` replacement.** The context fabric is a
   no-network Rust system daemon whose per-chunk ACLs, provenance tags,
   Ledger hooks, and user-openable single SQLite file are *security
   architecture* (PLAN §5.3, §5.10, §0.6 "Python only for build tooling
   and evals"). A Python service stack with three embedded engines
   (SQLite + LanceDB + Kuzu) can't take that seat, and shouldn't.

2. **Runs on Lisa unmodified, day one of M5.** PLAN §5.4 promises
   "thousands of existing MCP servers run on Lisa unmodified" — cognee is
   the ideal first third-party proof: register `cognee-mcp` as a
   user-level MCP server, point its LLM + embedding provider at our
   OpenAI-compat endpoint (`127.0.0.1:7777`), and a user gets a fully
   local knowledge-graph memory with zero cloud. **Action:** add cognee
   to the M5 third-party-server test matrix.

3. **Design ideas worth absorbing into §5.3 (M3 design phase):**
   - *Graph over chunks:* their core claim — relational/causal questions
     need graph traversal, flat embedding indexes can't answer them — is
     credible. Our hybrid search (BM25 + vector + recency + rerank) can
     grow an optional entity/relation layer as plain SQLite adjacency
     tables; no graph DB needed. Evaluate during M3, gated on real
     retrieval-quality wins in evals.
   - *Verb set:* `remember/recall/forget/improve` is a cleaner per-app
     memory API shape than bare KV get/put — worth considering for the
     `dev.lisaos.portal.Memory` surface.
   - *Consolidation:* their "whole memory layer on one Postgres" pitch
     validates our "whole context fabric in one user-readable SQLite"
     bet.

## Sources

- https://github.com/topoteretes/cognee
- https://www.cognee.ai/ and https://www.cognee.ai/blog/fundamentals/how-cognee-builds-ai-memory
