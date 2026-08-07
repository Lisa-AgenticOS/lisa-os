#!/usr/bin/env python3
"""Generate the OS knowledge pack (#175, ADR-0040).

The pack is what the on-device model retrieves when asked about the OS
it runs on ("how do I update?", "what does the Ledger show?"), indexed
by `lisa context sync-knowledge` under the `system` provenance. The
same output feeds lisaos.dev's docs build (ADR-0040: one curation
step, two consumers).

Sources are component READMEs — rule 10's component truth — from an
explicit, curated list. Curation is the point: a generator that slurps
the whole repo would feed the model ADR-internal reasoning in the
wrong register. Adding a component here is a one-line review-visible
decision.

Outputs are COMMITTED (docs/knowledge/), same trade as
branding/generate-tokens.py: the PKGBUILD copies a file tree, and
`--check` in the lint gate keeps source and output from disagreeing on
main. Stamping with the image version happens at index time by
content hash, not here — bytes cannot lie about themselves.
"""

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "docs/knowledge"

# (source README, output name, one-line "what is this about" used as
# the pack's index entry)
SOURCES = [
    ("apps/mail/README.md", "mail.md", "The Mail app: reading, searching and writing email"),
    ("apps/notes/README.md", "notes.md", "Notes: a daemon and a window sharing one tool surface"),
    ("apps/lisa.sdk/README.md", "lisa-sdk.md",
     "lisa.sdk: the shared GJS library every Lisa window is built on"),
    ("apps/surfer/README.md", "surfer.md", "Surfer, the browser"),
    ("apps/preview/README.md", "preview.md", "Preview: images, PDF, and Space-to-preview in Files"),
    ("apps/terminal-integration/README.md", "terminal.md", "lisa explain / lisa suggest in the terminal"),
    ("shell/assistant/README.md", "assistant.md", "The Lisa Assistant chat window"),
    ("shell/ledger-app/README.md", "ledger.md", "The Ledger: every AI action, auditable"),
    ("shell/overlay-extension/README.md", "overlay.md", "The assistant overlay and how to summon it"),
    ("cli/lisa/README.md", "cli.md", "The lisa command line"),
    # models/ has no top-level README (its children do, and they are
    # data-format docs, not user answers) — a models.md joins the pack
    # when someone writes the user-facing one. Rule 10: nothing here
    # documents what does not exist.
]

HEADER = (
    "<!-- GENERATED into the OS knowledge pack from {src} by\n"
    "     os/repo-tools/build-knowledge.py — edit the source README,\n"
    "     then regenerate. (#175, ADR-0040) -->\n\n"
)


def build():
    docs = {}
    missing = []
    for src, name, about in SOURCES:
        path = ROOT / src
        if not path.is_file():
            missing.append(src)
            continue
        docs[name] = HEADER.format(src=src) + path.read_text()
    if missing:
        print(f"knowledge: sources missing (fix the list or the tree): {missing}")
        return None
    manifest = {
        "docs": [
            {
                "file": name,
                "about": about,
                "sha256": hashlib.sha256(docs[name].encode()).hexdigest(),
            }
            for (_, name, about) in SOURCES
        ],
        "$comment": "GENERATED — see build-knowledge.py. Consumed by "
        "`lisa context sync-knowledge` and the lisaos.dev docs build.",
    }
    docs["manifest.json"] = json.dumps(manifest, indent=2) + "\n"
    return docs


def main():
    docs = build()
    if docs is None:
        return 1

    if "--check" in sys.argv:
        stale = [
            name for name, text in docs.items()
            if not (OUT / name).exists() or (OUT / name).read_text() != text
        ]
        if stale:
            print(f"knowledge: STALE outputs {stale} — run python3 os/repo-tools/build-knowledge.py")
            return 1
        print(f"knowledge: {len(docs) - 1} docs + manifest in sync with their READMEs")
        return 0

    OUT.mkdir(parents=True, exist_ok=True)
    for name, text in docs.items():
        (OUT / name).write_text(text)
        print(f"wrote docs/knowledge/{name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
