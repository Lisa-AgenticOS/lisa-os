# lisa-ledger — the append-only record

Spec: `docs/PLAN.md` §5.10, `docs/VISION.md` ("radical legibility").

## What it does

Every time Lisa acts, it says so here, permanently.

The Ledger is not a log — it is the product feature that makes always-on
intelligence acceptable. `VISION.md`: *"If a Golden-Gate user asks 'what
did Siri actually read?' there is no answer; on Lisa the Ledger **is** the
answer."* Always-on is only tolerable because it is always legible.

Two rules follow from that, and they are enforced rather than encouraged:

- **Append-only.** Entries are never updated or deleted.
- **No ledger entry, no action.** Callers append *before* they act, and a
  failed append aborts the action. `agentd` refuses to start without a
  Ledger; the forge loop aborts a run it cannot record.

## How it works

SQLite, one table, opened at `Ledger::default_path()`.

```rust
let ledger = Ledger::open(Ledger::default_path())?;
let id = ledger.append(&Event {
    kind: "tool.call".into(),
    app_id: actor.into(),
    preview: preview_of(&summary),
    status: "started".into(),
    ..Default::default()
})?;
// … do the thing …
ledger.append(&Event { status: "ok".into(), ref_id: Some(id), ..Default::default() })?;
```

`ref_id` links an outcome back to the intent that preceded it, so a
reader can pair them and spot an intent that never completed.

`preview_of` truncates human-readable text for display.

## How to extend it

**Adding an event kind** is just a new `kind` string — there is no
registry. Use `<component>.<verb>`: `tool.call`, `tool.confirm`,
`remote.generate`, `forge.tool`, `boot.repair`, `dev.install`. Pair every
intent with an outcome carrying `ref_id`.

**Before you append, ask what you are putting in `preview` and
`detail`.** These are the fields most likely to leak, because the natural
thing to write is "whatever the tool returned".

## Limits and open issues

- **Secrets can be written into it, and it cannot be redacted** (#127).
  The forge loop previews tool arguments and outputs, so `read_file .env`
  copies credentials into an append-only store. Append-only is the whole
  point, which is exactly why what goes in matters more here than
  anywhere else.
- **Control characters survive** into a reader's terminal (#128) — issue
  #15's lesson was applied to model output reaching the shell, but not to
  the Ledger.
- **`actor` is caller-asserted** (#55), so entries record reliably *that*
  something happened and unreliably *who* did it, until ADR-0033 lands
  everywhere.
- `preview_of` caps at 160 **chars**, which is up to 640 bytes; its test
  asserts `len() <= 160` and so passes only on ASCII.

## Readers

`lisa ledger [--tail N] [--json]`, the Ledger app (`shell/ledger-app`),
and the Settings panel.
