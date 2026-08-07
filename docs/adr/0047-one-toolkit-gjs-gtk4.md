# ADR-0047 — One toolkit: GJS + GTK4/Adwaita is the default, Flutter is parked

- **Status:** accepted — GJS + GTK4/Adwaita is the documented default, #37
  is closed won't-do, and PLAN §5.8/§5.12 and ADR-0004 carry the correction.
  Not yet done: `lisa_ui` becoming the shared GJS library — the MCP edge it
  is meant to de-duplicate still exists in triplicate (`apps/mail`,
  `apps/preview` and `apps/surfer` each carry their own `lib/mcp.js` and
  `lib/mcp-protocol.js`). Corrected 2026-08-06: §2 below parks
  `libs/lisa_ui` and `libs/lisa_flutter` "in the tree" and they were
  deleted on 2026-08-06 (`d1bdc18`); the name is reserved and the directory
  does not exist. ADR-0056 is what it becomes.
- **Amended 2026-08-07 (owner call): the Flutter lane went from parked to
  REMOVED.** §2's "parked, not deleted" no longer holds. Deleted:
  `lisa forge --flutter`/`--setup`/`--build`/`--run` and the pinned-SDK
  provisioning in `cli/lisa`, the harness's flutter tooling (`flutter
  test` dispatch, the SDK `PATH` dirs, the Landlock read grant on
  `/var/lib/lisa/flutter`), the guard's `flutter` allowlist entry and
  policy with their corpus rows (a deny row now proves the reverse), and
  the `forge/app` scaffold placeholder. Nothing user-facing was ever
  built with the lane; its history stays in git and in ADR-0004/0014/0027.
- **Date:** 2026-08-04
- **Supersedes:** ADR-0004's "Flutter lane" as the *default* for
  user-facing apps and for Forge output. PLAN §5.12 and §314 need the same
  correction.
- **Related:** ADR-0038 (design tokens), ADR-0020 (apps channel), #37
  (ship the Flutter lane on-device — to be closed by this decision), #51
  (Forge).
- **Claims:**
  - `path:apps/mail/lib/mcp.js` — the MCP edge, copy 1 of 3
  - `path:apps/preview/lib/mcp.js` — copy 2
  - `path:apps/surfer/lib/mcp.js` — copy 3, which is what "still exists in triplicate" means
  - `absent:libs/lisa_ui` — and the library meant to de-duplicate them does not exist

## Context

ADR-0004 named Flutter "the default framework for user-facing apps,
third-party apps, and everything the Forge generates." That was a
reasonable bet when nothing user-facing existed yet.

Ten months of apps later, the bet has been answered by practice. Every
shipped surface is GJS with GTK4/Adwaita:

- `shell/assistant`, `shell/launcher`, `shell/desktop`, `shell/overlay-extension`,
  `shell/consent`, `shell/ledger-app`
- `apps/mail`, `apps/surfer`, `apps/preview`, and the terminal integration

Nothing user-facing has ever been written in Flutter. `libs/lisa_ui` and
`libs/lisa_flutter` measure 45 MB on disk, but that is `build/` output —
the actual source is **16 KB of `lib`, 8 KB of `test`, four `.dart` files
between them**. #37 (ship the Flutter SDK and `lisa_ui` on-device) has
never been done, so the lane has no runtime on the reference hardware at
all.

So the plan says one thing and the system is another, which CLAUDE.md
rule 3 says to resolve with an ADR rather than let drift continue.

## Why the practice went this way

It was not an accident, and the reasons are the argument:

**Iteration on real hardware.** GJS is interpreted, so a fix reaches the
reference iMac by copying files — no image rebuild, no cross-compile. On
2026-08-04 every fix in a batch of five (Mail's binary-body defect,
Surfer's world isolation, the socket timeouts) was deployed by `scp` and
verified against the real device in minutes. A compiled lane makes that
loop an image build.

**Desktop integration is the product.** These apps live or die on portals,
D-Bus activation, AT-SPI (which is how surfaces get verified when
screenshots are blocked), dconf, `.desktop` activation, dock state, and
input methods. GTK4/Adwaita gets those for free because it *is* the
platform; Flutter on Linux reaches them through shims, and its
accessibility and IME stories are the weakest parts. For a distribution
whose input-method work is already hard (#191, #208), adopting a toolkit
with a thinner IME story is the wrong direction.

**One toolkit is one of everything else.** One design-token sheet
(ADR-0038), one test harness (`shell/testing/harness.js`), one review
surface, one set of idioms for a reviewer to hold in their head. The
2026-08-04 review found 27 defects across three apps precisely because
they shared enough shape to be reviewed together.

**The SDK already covers other languages.** liblisa exposes a C ABI with
Rust, Python, JS and Vala bindings plus an OpenAI-compatible endpoint, so
"an app framework we do not ship" was never the thing standing between a
third-party developer and Lisa.

## Decision

1. **GJS + GTK4/Adwaita is the default and documented framework** for
   Lisa's own user-facing apps and for everything the Forge generates.
2. **Flutter is parked, not deleted.** `libs/lisa_ui` and
   `libs/lisa_flutter` stay in the tree, and their READMEs state plainly
   that the lane is unshipped, unproven on hardware, and not the default —
   rather than describing a lane that works.
3. **#37 is closed as won't-do** under this ADR. Shipping a Flutter SDK to
   `/var` for a lane with no app is payload nobody uses.
4. **The Forge targets GJS.** Generated code should be the same shape as
   hand-written code, so a generated app is reviewable by the same people
   with the same instincts — and an interpreted target means the Forge can
   produce something runnable without a build toolchain (#48).
5. **PLAN §5.12 and §314 and ADR-0004 are corrected** to describe this,
   because a plan that names a default nobody uses is the defect rule 10
   warns about.
6. **`libs/lisa_ui` becomes the GJS/GTK4 shared library** — the same name,
   pointed at the toolkit we actually use. It owns what every app is
   currently re-implementing, and its first job is to end the duplication
   that is already costing us bugs (below).

## The duplication this is meant to end

There is no shared library today, so each app carries its own copy of the
same modules. Counted on 2026-08-04:

| module | copies |
|---|---|
| `mcp-protocol.js` | 3 (mail, surfer, preview) |
| `mcp.js` | 3 |
| `model.js` | 3 |
| `attachments.js` | 2 |
| `actions.js` | 2 |

This is not a tidiness argument. **Issue #218 — a tool dispatcher that
resolved names through `Object.prototype` and answered *success* for
`constructor` instead of `-32601` — existed in all three copies of
`mcp-protocol.js` and had to be found once and fixed three times.** A
review that had only looked at Surfer would have left the same fail-open
hole live in Mail and Preview. One library would have made that one fix,
and one corpus entry.

So `lisa_ui`'s scope, in the order the evidence justifies:

1. **The Agent Bus edge** — `mcp-protocol.js` / `mcp.js`. Highest value,
   because it is a security boundary that currently exists in triplicate.
   The socket lifecycle belongs here too: #219 (a killed app leaving a
   socket that refuses connections while the bus treats presence as
   availability) was likewise found in two apps separately.
2. **Design tokens** — already generated from `branding/tokens.json`
   (ADR-0038) and enforced by `check-tokens.py`. The generated GJS sheet
   moves here rather than being copied per app.
3. **Common widgets**, only once a second app needs one. Not a speculative
   component set — the attachment row exists in Mail alone today, and
   pulling it up before Preview or the Assistant needs it would be
   inventing a contract from one example.

Migration is incremental and per-module: an app moves to the shared module
when someone is already touching that file, not as a flag-day rewrite.
Each move must keep the app's tests green without rewriting them — a
shared module that forces every caller's tests to change is not shared,
it is a second implementation with extra steps.

The Flutter package that held this name keeps its four `.dart` files under
`libs/lisa_flutter`, so nothing is deleted and the history is intact.

## What would reverse this

Stated up front so a future session can weigh it honestly rather than
re-arguing from scratch:

- **A shipped app that GTK4 genuinely cannot serve** — heavy custom
  rendering, a canvas-style editor, sustained 60fps animation. Nothing
  built so far comes close.
- **GJS hitting a wall we cannot design around.** The known one is
  performance on large data: the mail list froze on a 3,758-message inbox
  until paging was added. Paging fixed it, but a second wall of that kind
  would be evidence.
- **A third-party developer story that demands Dart specifically** —
  unlikely while liblisa carries five bindings.

None of those is true today, and the cost of reversing is four `.dart`
files.

## Consequences

**We lose the cross-platform option we were never using.** Flutter would
have made a Lisa app portable to other systems. That is not a goal of a
Linux distribution whose thesis is the integration of the local system.

**The installer inherits this.** ADR-0046's storefront and the live-USB
installer (#100) target GTK4/Adwaita rather than copying Ubuntu's
Flutter-based installer. The installer *engine* stays headless Rust with a
TTY path regardless of toolkit, so this choice stays cheap and reversible
at the surface.

**Typing discipline is now our problem to solve.** Dart's static types
were a real advantage that GJS does not give us. The mitigation is the one
already in force: pure logic in testable modules, a house rule of
failing-test-first, and mutation checks — the practice that caught #210
and #221. That is a weaker guarantee than a type system, and saying so is
part of accepting this trade.
