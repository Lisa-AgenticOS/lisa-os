# lisa-contextd — context fabric

Spec: docs/PLAN.md §5.3. Milestone: M3.

System-wide personal context index (files, mail, calendar, chat, screen — each individually consented) plus per-app durable memory. SQLite + FTS5 + sqlite-vec, encrypted at rest, every retrieval ledgered with provenance tags and per-chunk ACLs.

Status: **not started** — scaffold placeholder. Read the spec section (and CLAUDE.md rules) before writing code here.

Prior art: [cognee evaluation](../../docs/notes/cognee-evaluation.md) — knowledge-graph memory platform; not the substrate (Python, multi-engine), but a flagship M5 MCP tenant and a design reference for the M3 entity-graph question.
