# ADR-0043: The model knows the OS through retrieval, never through the prompt

- **Status:** accepted, partially executed — phase 1 shipped (#175: the
  pack, the generator, `system` provenance, `lisa context sync-knowledge`,
  the session-start unit), and answers were verified semantically on the
  device. Open: retrieval wiring in the assistant and overlay lanes,
  `--help` in the pack, and the on-device answer-quality eval.
- Date: 2026-08-03
- Relates: ADR-0040 (docs live with the code; one generator, two
  consumers), ADR-0029/0030 (guardrail boundary), #175, #176 (the
  recall work this depends on), rule 6a/6b

## Context

The owner asked: *"Should we update harness with knowledge of os so
models will know everything about it?"* The obvious implementation —
OS documentation in the system prompt — is wrong three ways on this
hardware: the local models are a 1.7B/4B where every prompt token is
latency; the corpus that exists (PLAN, ADRs) is internal reasoning in
the wrong register for user answers; and prompt text is the *trusted*
channel, so anything that rides it inherits authority it should not
have.

## Decision

1. **Knowledge is data, retrieved on demand.** A curated pack
   (`docs/knowledge/`, generated from component READMEs — rule 10's
   component truth — by `os/repo-tools/build-knowledge.py`, lint-gated
   against staleness) ships at `/usr/share/lisa/knowledge` and is
   indexed into the context store. Answers about the OS come from
   hybrid search over it, like answers about the user's own files.
2. **It describes the running image, never the newest docs.** The pack
   travels only in the image/runtime channels; sync
   (`lisa context sync-knowledge`) detects change by content hash —
   bytes cannot disagree with themselves — prunes what an upgrade
   removed, and embeds only via the model-backed embedder (mixing
   hash-fallback vectors into a model-vector store makes cosine noise
   that looks like ranking).
3. **`system` is a provenance, and it is read-tier.** The pack
   *informs* answers; it never *authorizes* actions. A doc chunk
   saying "run `lisa update`" does not skip a confirmation the guard
   would demand — rule 6a unchanged. This is also why retrieval beats
   prompt-stuffing on safety: text injected into a doc lands in the
   provenance-tagged untrusted lane, not the trusted prompt. Every
   scope that can read anything may read `system` — the pack is public
   bytes on the machine; a dedicated consent scope would be theater.
4. **The system prompt stays a paragraph**: identity, version, and
   "consult retrieved system docs for OS questions."

## Consequences

- The ADR-0040 bargain is now load-bearing both ways: the same
  curation feeds the on-device model and lisaos.dev, so a README lie
  becomes a model lie becomes a website lie — one gate stops all
  three.
- Adding a component to the pack is a one-line, review-visible edit to
  the generator's source list; `models.md` is absent because its
  user-facing README does not exist yet (rule 10).
- Answer quality on the 1.7B with retrieved context is still an
  empirical question. The eval happens on the device, and if quality
  is poor the fix is corpus and retrieval work — the architecture does
  not move back into the prompt.
