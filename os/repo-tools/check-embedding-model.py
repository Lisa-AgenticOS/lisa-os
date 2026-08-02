#!/usr/bin/env python3
"""contextd's EMBEDDING_MODEL must be the catalog's embeddings model.

`models/catalog/catalog.toml` is the source of truth for what each model
is FOR (`task = "embeddings"`). contextd cannot read it at runtime — it
is build-time data — so the id is duplicated as a constant in
`daemons/contextd/src/embed.rs`.

A duplicate that nothing checks is a duplicate that drifts, and the way
it would fail is the worst kind: `preferred_model()` simply never finds
its match, the embedder silently falls back to the daemon's default chat
model, and retrieval quality quietly degrades with nothing in any log
saying the constant went stale. That is issue #163's own failure mode
reappearing one level up.

Run by `just lint`.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CATALOG = ROOT / "models" / "catalog" / "catalog.toml"
EMBED_RS = ROOT / "daemons" / "contextd" / "src" / "embed.rs"


def catalog_embedding_ids() -> list:
    """Every [[model]] whose task is embeddings."""
    text = CATALOG.read_text()
    ids = []
    for block in text.split("[[model]]")[1:]:
        # Stop at the next top-level table so a later section cannot leak in.
        block = block.split("\n[", 1)[0]
        task = re.search(r'^\s*task\s*=\s*"([^"]+)"', block, re.M)
        ident = re.search(r'^\s*id\s*=\s*"([^"]+)"', block, re.M)
        if task and ident and task.group(1) == "embeddings":
            ids.append(ident.group(1))
    return ids


def main() -> int:
    if not CATALOG.is_file() or not EMBED_RS.is_file():
        print(f"FAIL: expected both {CATALOG} and {EMBED_RS}")
        return 1

    m = re.search(r'EMBEDDING_MODEL:\s*&str\s*=\s*"([^"]+)"', EMBED_RS.read_text())
    if not m:
        print("FAIL: no EMBEDDING_MODEL constant in embed.rs — this check is checking nothing")
        return 1
    constant = m.group(1)

    ids = catalog_embedding_ids()
    if not ids:
        print('FAIL: no catalog entry with task = "embeddings" — parsed nothing to compare')
        return 1

    if constant not in ids:
        print(f"FAIL: contextd names '{constant}', which is not a catalog embeddings model.")
        print(f"      catalog task=\"embeddings\": {', '.join(ids)}")
        print("      A stale constant does not error — the embedder just never finds its")
        print("      model and silently falls back to a chat model (#163).")
        return 1

    print(f"embedding model: ok ({constant} is task=\"embeddings\" in the catalog)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
