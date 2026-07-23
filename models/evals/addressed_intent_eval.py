#!/usr/bin/env python3
"""Addressed-intent eval — the Phase-2 gate for wake-word-free Ambient
(ADR-0011, PLAN §11). Labeled utterances → run the classifier → report
accuracy and, most importantly, the **false-accept rate**: how often
un-addressed speech is wrongly treated as a request. Responding when not
addressed is the failure that makes always-on creepy, so that's the
number the gate watches.

    python addressed_intent_eval.py            # against 127.0.0.1:7777

Wake-word mode ("Hey Lisa") ships by default; Ambient goes wake-word-free
only when a model clears this gate (target: false-accept < 5%).
"""
import json
import sys
import urllib.request

URL = "http://127.0.0.1:7777/v1/chat/completions"

# (utterance, addressed?) — a small labeled set; grow before gating.
FIXTURES = [
    ("Hey Lisa, what's the weather tomorrow?", True),
    ("Lisa, add milk to my shopping list", True),
    ("could you turn the volume down", True),
    ("what time is my next meeting", True),
    ("summarize this document for me", True),
    ("open the file I was just editing", True),
    ("I think we should order pizza tonight", False),
    ("did you see the game last night?", False),
    ("the mona lisa is in the louvre", False),
    ("let me call you right back", False),
    ("ugh, where did I put my keys", False),
    ("he said the meeting moved to three", False),
    ("this coffee is way too strong", False),
    ("can you believe how cold it is", False),
]

SYSTEM = (
    "You are the ear of Lisa, an on-device assistant. Decide whether a "
    "transcribed utterance is addressing Lisa (a request to do or answer) "
    "versus talking to a person, thinking aloud, or background speech. Be "
    "conservative: when unsure, addressed is false. Reply ONLY as JSON."
)
SCHEMA = {
    "type": "object",
    "properties": {
        "addressed": {"type": "boolean"},
        "confidence": {"type": "number"},
        "intent": {"type": "string", "maxLength": 200},
    },
    "required": ["addressed", "confidence", "intent"],
}


def classify(text: str) -> bool:
    body = json.dumps({
        "model": "lisa", "max_tokens": 200,
        "messages": [{"role": "system", "content": SYSTEM},
                     {"role": "user", "content": text}],
        "response_format": {"type": "json_schema",
                            "json_schema": {"name": "addressed", "schema": SCHEMA}},
    }).encode()
    req = urllib.request.Request(URL, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        content = json.load(r)["choices"][0]["message"]["content"]
    return bool(json.loads(content)["addressed"])


def main() -> int:
    correct = false_accept = false_reject = 0
    n_neg = sum(1 for _, a in FIXTURES if not a)
    for text, expected in FIXTURES:
        got = classify(text)
        ok = got == expected
        correct += ok
        if got and not expected:
            false_accept += 1
        if not got and expected:
            false_reject += 1
        print(f"  [{'ok ' if ok else 'MISS'}] want={int(expected)} got={int(got)}  {text}")
    n = len(FIXTURES)
    n_pos = n - n_neg
    fa_rate = false_accept / n_neg if n_neg else 0.0
    recall = (n_pos - false_reject) / n_pos if n_pos else 0.0
    print(f"\naccuracy {correct}/{n} = {correct/n:.0%}  |  "
          f"false-accept {false_accept}/{n_neg} = {fa_rate:.0%}  |  "
          f"recall {n_pos - false_reject}/{n_pos} = {recall:.0%}")
    # A usable wake-word-free gate needs BOTH: rarely fires when not
    # addressed (privacy), AND actually responds when addressed (utility).
    # "Reject everything" gets 0% false-accept but is useless — recall
    # guards against that.
    passed = fa_rate < 0.05 and recall > 0.70
    print(f"gate (Phase-2 wake-word-free): false-accept < 5% AND recall > 70% — "
          f"{'PASS' if passed else 'not yet (ship Hey Lisa)'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
