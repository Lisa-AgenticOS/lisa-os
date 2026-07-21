#!/usr/bin/env python3
"""M0 backlog item (PLAN Appendix D): the OpenAI-compat endpoint must work
with the unmodified OpenAI Python client — non-streaming and streaming —
against a running lisa-inferenced. Forerunner of the §5.1 acceptance line
"curl ... with an unmodified OpenAI Python client works"."""

import sys

from openai import OpenAI

client = OpenAI(base_url="http://127.0.0.1:7777/v1", api_key="local-no-key-needed")

models = client.models.list()
assert models.data[0].id == "lisa-system-stub", models

r = client.chat.completions.create(
    model="lisa-system-stub",
    messages=[{"role": "user", "content": "openai-client-canary"}],
)
content = r.choices[0].message.content
assert "openai-client-canary" in content, content
assert r.choices[0].finish_reason == "stop", r

stream = client.chat.completions.create(
    model="lisa-system-stub",
    messages=[{"role": "user", "content": "openai-stream-canary"}],
    stream=True,
)
streamed = "".join(
    chunk.choices[0].delta.content or ""
    for chunk in stream
    if chunk.choices
)
assert "openai-stream-canary" in streamed, streamed

emb = client.embeddings.create(model="lisa-system-stub", input=["alpha", "alpha", "beta"])
assert len(emb.data) == 3, emb
assert emb.data[0].embedding == emb.data[1].embedding
assert emb.data[0].embedding != emb.data[2].embedding

print("OPENAI CLIENT: PASS")
sys.exit(0)
