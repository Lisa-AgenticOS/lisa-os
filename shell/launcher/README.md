# Semantic launcher & search

Spec: docs/PLAN.md §5.7.2. Milestone: M4.

One box mixing app launch, lexical+vector file hits, bus actions, and
grammar-constrained calculator answers (math routes to qalc, never the
model). Budgets: first results < 150 ms, semantic refinement < 700 ms.

## Layout

- `extension.js` + `metadata.json` — GNOME Shell extension (ESM,
  GNOME 46+) registering a search provider that *augments* Shell
  search: GNOME's providers keep the app lane; ours adds
  - **"Ask Lisa" (assistant handoff)**: every query ≥ 2 chars gets an
    entry that hides the overview and activates the `ask` action on
    `app.lisaos.Assistant` over `org.gtk.Actions` — the conversation
    opens in the Assistant, in a new session, starting the app if it is
    not running (Spotlight-style; promoted above file hits when the
    query reads like a question). Icon: bundled `lisa-mark.svg`;
  - **calculator/unit answers**: conservative routing heuristic →
    `qalc -t` subprocess → answer as the first result (Enter copies);
  - **file hits**: `lisa context search` (Context Fabric FTS5, PLAN
    §5.3 — the CLI ledgers every retrieval), snippet as description,
    Enter opens with the default app.
- `lib/ranking.js` — pure routing/merge/id logic (no GNOME imports).
- `lib/summon.js` — what Super+Space does given what is on screen.
- `schemas/org.gnome.shell.extensions.lisa-launcher.gschema.xml` — the
  keybinding, `toggle-search`, defaulting to `['<Super>space']`.
- `tests/ranking.test.js`, `tests/summon.test.js` — unit tests
  (`just shell-test`).

## The summon key

**Super+Space** shows the overview with the caret already in the search
entry, and dismisses it when the search is what is on screen. From a
window-picker overview it moves into the search rather than closing it:
the chord is a search key, not an overview toggle.

It is a default, not a lock — Settings › Keyboard rebinds it and the
change sticks, because no `locks/` directory ships.

Adding a key here means adding it in two places: the schema, and a
`Main.wm.addKeybinding` / `removeKeybinding` pair in `extension.js`.
`os/repo-tools/check-shell-keys.py` (run by `just lint`) fails if a
schema key is never bound, if the schema is not compiled by the
package, or if two things Lisa ships claim the same chord — issue #255
was this chord being reserved in three comments and bound by nobody.

## Status

Working first pass. Deferred to their owning milestones: bus actions
("rotate this pdf") need `lisa-agentd` (M5, §5.4); semantic vector
refinement needs contextd's embedding pipeline (§5.3, M3 remainder);
the < 150 ms / < 700 ms budgets are enforced by the perf gate on
reference hardware (§11), not asserted on dev hosts.

Known limit, not yet filed: the app channel (`lisa apps update`, which
unpacks `lisa-apps_<ver>.tar.zst` onto `/var`) does not run
`glib-compile-schemas` over the tree it installs — only the package
build does. An extension delivered that way has an uncompiled
`schemas/`, so `getSettings()` throws. This predates the launcher's
schema; the overlay has the same exposure.

Install (dev): `glib-compile-schemas schemas/`, symlink into
`~/.local/share/gnome-shell/extensions/lisa-launcher@lisa-os.org`,
re-log. Skipping the compile makes `getSettings()` throw and the
extension never enables. Needs `libqalculate` (qalc) and an indexed
context store (`lisa context index ~/Documents`).

Install (packaged): ships in the `lisa-shell` package
(os/packages/lisa) — tree under `/usr/share/lisa/shell/`, extension
symlink, qalc via the `libqalculate` dependency, default-enabled by
the package's gschema override. The Track I release image folds it in.
