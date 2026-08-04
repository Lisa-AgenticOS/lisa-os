# ADR-0050 — App tooling is CLI verbs, and the scaffold carries the traps

- **Status:** Accepted (decision only — **no code exists**; see
  "What exists today" and "What this ADR decides")
- **Date:** 2026-08-04
- **Related:** ADR-0047 (GJS + GTK4 is the one toolkit), ADR-0029/0030
  (the guardrail boundary), ADR-0034 (`lisa dev`, `$HOME` not root),
  ADR-0048 (core vs. store), ADR-0049 (every app is an agent surface),
  PLAN §5.12.1 (the Forge loop), `docs/ANATOMY-OF-AN-APP.md`
- **Issues:** #48, #130, #218, #219, #239, #240, #241, #212, #210, #223
- **Supersedes nothing.** It decides the *shape* of app-authoring tooling,
  which no ADR had claimed.

## Context

We are not building an IDE. No GUI, no project file, no editor
integration, no workspace concept. The deliverable is a small set of
`lisa` verbs that a person and the Forge harness invoke identically —
scaffolding via CLI, so the harness can one-shot an app.

That framing sounds like a convenience decision. It is not, and the
reason is in one document.

### The traps are all documentation

`docs/ANATOMY-OF-AN-APP.md` §7 lists five defects that shipped. Every one
of them is, today, **a line of prose asking a reader to remember
something**:

| issue | what it cost | where it lives now |
|---|---|---|
| **#241** | Preview's manifest installs to `/usr/share/lisa/apps/` while `SYSTEM_MANIFEST_DIR` is `/usr/share/lisa/manifests` (`daemons/agentd/src/main.rs:17`). A shipped core app's declared tools have **never reached the model**, and nothing errored, warned or logged | `ANATOMY §7`, `os/packages/lisa/PKGBUILD:391-392` |
| **#218** | `tools[name]` resolves through `Object.prototype`, so `constructor` was *called* and answered a tagged **success** where the protocol says `-32601`. It existed in **three** copies of `mcp-protocol.js` | `ANATOMY §7`, ADR-0047 §"The duplication this is meant to end" |
| **#219** | the MCP socket released on `shutdown` alone, so a killed app leaves a socket that refuses connections while mcp-bus reads presence as availability. Still live in `apps/surfer/lisa-surfer.js:723` and `apps/preview/lisa-preview.js:1444-1447` | `ANATOMY §3` |
| **#212** | agent scripts evaluated in the page's own JS world, where the page owns `JSON.stringify` and `document.querySelector` — an approved `fill` on `#q` landed in `#pw` | `ANATOMY §7` |
| **#210 / #223** | `?? fallback` swallowing a null: every reply this app composed had no `In-Reply-To`; every message opened to an empty reading pane | `ANATOMY §7` |

Plus the one that produces no log line at all: **top-level `await` in an
entry module**, which binds the socket, advertises it, and answers
nothing. It cost this repo twice — Mail and Preview.

A document is the right place to *explain* these. It is the wrong place
to *enforce* them, and the evidence is that we wrote them all down and
#219 is still open in two apps.

### What exists today, verified

Stated precisely, because the temptation in a tooling ADR is to describe
the tool as if it were already there (CLAUDE.md rule 10).

- **`lisa forge` is built.** `cli/lisa/src/main.rs:1525` (`forge_cmd`)
  drives `libs/forge-harness`: the model writes a whole file, the tool
  jail (`libs/forge-harness/src/jail.rs`) confines writes to the project
  directory, a verifier runs, findings feed back, repeat. The loop calls
  the verifier — the model does not, and `DoneClaimed` is followed by a
  check, not accepted.
- **"Passes analysis" means `dart analyze`.**
  `libs/forge-harness/src/agent.rs:108-112` — `Verifier` has exactly
  three arms: `Dart`, `Command { program, args }`, `None`.
  `forge_cmd` picks `Command { flutter analyze }` under `--flutter` and
  **`Verifier::Dart` otherwise**, after writing a `pubspec.yaml`
  (`main.rs:1559-1571`). There is no GJS arm and no GJS analyzer in the
  tree.
- **So `lisa forge` cannot forge a GJS app at all today.** A run without
  `--flutter` gets `Verifier::Dart`, which reports *"the project contains
  no Dart source files yet"* (`agent.rs:125-135`) and can never converge
  on JavaScript. ADR-0047 §4 says "the Forge targets GJS"; the code still
  scaffolds Flutter. That gap is not this ADR's to close, but it is the
  reason the verb below is load-bearing.
- **`lisa skills` is built** (`cli/lisa/src/skills.rs`): a system-wide
  search path (`$LISA_SKILLS_DIR`, `$XDG_DATA_HOME/lisa/skills`, the
  runtime channel, `/usr/share/lisa/skills`), earlier wins on a name
  clash. `skills/build-lisa-ui-app/SKILL.md` still describes the Flutter
  lane and is being rewritten for GJS.
- **There is no `lisa new` and no `lisa dev`.** The `Command` enum in
  `cli/lisa/src/main.rs:33-350` has neither. Nothing in this ADR
  describes behaviour that exists.
- **Two writer/reader agreements are already mechanized in `cargo test`**
  — `cli/lisa/tests/apps_payload.rs` and `cli/lisa/tests/apps_launcher.rs`.
  They exist because for three releases the launcher's path list and the
  installer's had drifted one directory apart (#239). That is the
  precedent this ADR reuses.

## Decision

### 1. CLI verbs under `lisa`, and nothing else

No GUI, no project file, no editor plugin, no `lisa-*` helper script
(CLAUDE.md rule 7 — one command center). The reasons are not stylistic:

- **The harness can already reach a verb, on terms that are already
  settled.** When the model runs one, `libs/forge-harness/src/shell_tool.rs`
  applies four non-negotiable conditions — jailed, guard-checked through
  `lisa_guard::check_shell_line`, never silent, never unattended (no
  constructor without a consent callback). When the *loop* runs one as a
  verifier it is plain argv — `Command::new(program).args(args)`,
  `libs/forge-harness/src/agent.rs:136-143` — not a shell line the model
  composed, so `lisa check` is not even reachable by the surface that has
  leaked in four consecutive review rounds. Either way there is one entry
  point to secure, and it is secured.
- **A GUI would need a second implementation of every check**, and a
  check that exists twice is #218 again.
- **One binary is one place to look.** `lisa doctor` already collects the
  machine's state; an app-authoring surface belongs beside it, not in a
  window.

### 2. Two verbs. That is the whole surface.

**`lisa new app <Name>`** — write a runnable app tree under `apps/<name>/`
(or a given directory): entry point, `lib/`, `tests/`, manifest,
`.desktop`, README stub. It emits the decisions §7 currently asks a
reader to remember: the `#218` own-property-and-callable dispatch, the
three idempotent socket releases (#219), the constant provenance tag with
the spread *before* it, the no-top-level-`await` comment, and the suite
that mutation-checks all three — the exact tests
`ANATOMY §9` already wrote and ran.

**`lisa check [path]`** — run everything mechanical over an app tree and
exit non-zero: the app's own suite, the manifest through **agentd's own
parser** (`daemons/agentd/src/manifest.rs`, never a second
implementation), the install-destination check that would have caught
#241, the entry-module checks, and `check-tokens.py`.

**`lisa check` is the load-bearing one.** A generator without a checker
produces plausible code faster. The checker is what closes the Forge
loop, and it needs no new harness code: `Verifier::Command { program:
"lisa", args: ["check"] }` is the arm that already exists
(`agent.rs:110`).

#### What we rejected, and why

- **A `run` verb.** GJS is interpreted (ADR-0047), so `gjs -m
  apps/<name>/lisa-<name>.js` already runs it on a device and
  `/usr/bin/lisa-app` already resolves the installed tree through the
  single authority in `cli/lisa/src/apps.rs:28-34`. A third path that
  spells the same location is precisely #239's defect, re-created on
  purpose.
- **A `package` verb.** Per-app packaging does not exist. ADR-0048 §5:
  *"one `shell` tarball, one version — that is an updater, not a store."*
  A `lisa package` today would either shell out to the PKGBUILD or invent
  a format with no consumer, and documenting it would be documenting
  intent as behaviour. Deferred until ADR-0046's catalog has something to
  install.
- **An `add tool` / `add window` generator family.** One example is not a
  contract. ADR-0047 §6.3 makes the same call about widgets: pull a
  shared thing up when a second caller needs it, not before.
- **A project/workspace abstraction.** An app is a directory you can
  `cp -a`. `os/repo-tools/build-apps-payload.sh` copies directories; that
  must stay true, so nothing may become a *prerequisite* of running an
  app.

### 3. The check is authoritative; the scaffold and the docs derive from it

A scaffold, a skill and a document all describing the same app shape is
three copies of one truth, and this repo has already paid for that
arrangement twice: #218 in triplicate, and #239 where the writer and the
launcher spelled the same path differently for three releases.

So:

1. **`lisa check` is the single authority** on what a valid Lisa app is.
   Where a rule is already enforced by shipped code — the manifest
   grammar, the tier floor, the token gate — `lisa check` **calls that
   code** rather than restating it.
2. **The scaffold is tested against the checker.** A test in
   `cli/lisa/tests/` scaffolds into a temp directory and asserts
   `lisa check` is clean, the same way `apps_payload.rs` asserts every
   `Exec=` resolves inside the staged tree. A scaffold that emits
   something its own checker rejects is a build failure, not a surprise
   on someone's afternoon.
3. **`skills/build-lisa-ui-app` and `ANATOMY §Checklist` stop restating
   the rules** and instruct: scaffold, then `lisa check`. They keep the
   *why* — the narrative in §7 is the most valuable thing in the tree and
   is not replaceable by a linter — and drop the imperative checklist that
   is a fourth copy waiting to drift.

That ordering is deliberate: the checker is the copy that **fails**, and
a copy that cannot fail is the one that goes stale.

### 4. This does not govern #130, and #130 does not gate it

ADR-0034 and issue #130 answer *"where does a third-party toolchain live
on an immutable root"* — `lisa dev install mysql`, rootless podman, a
pinned image under `$HOME`. Its phase 0 (subuid/subgid, `newuidmap`, the
podman runtime set) landed 2026-08-01; phase 1 is not started and is
sequenced behind manifest signing and the ADR-0033 rollout.

**Writing a Lisa app needs none of that.** `gjs`, GTK4 and Adwaita are in
the image, and an app is interpreted source. So `lisa new` / `lisa check`
are **separate from `lisa dev`, not a phase of it**, and inherit none of
its sequencing. If an app author wants Postgres to develop against, that
is `lisa dev`, unchanged and undisturbed by this record.

Two of ADR-0034's rules do carry over, because they are general:

- **`$HOME`, not root, and no `sudo` anywhere.** Nothing in these verbs
  writes outside the project directory. `escalate.privilege` is an
  unoverridable `Deny` in our own guard, and an authoring path that needed
  a carve-out would be arguing against a rule we spent a day defending.
- **The dependency rule does *not* bind here, and this is easy to
  over-apply.** ADR-0034 §1 constrains the *install, update and recovery*
  paths. Authoring is none of those; a developer with no `lisa check` has
  a worse afternoon, not an unbootable machine. These verbs may therefore
  depend on whatever serves them best — including, later, a third-party
  JS analyzer. What they may **not** do is become load-bearing for
  running or shipping an app.

### 5. Interpreted source is why one-shot is practical (ADR-0047)

ADR-0047 §4 chose GJS partly so "the Forge can produce something runnable
without a build toolchain (#48)". This ADR is where that pays:

- **The scaffold's output runs immediately.** No `pub get`, no analyzer
  install, no cross-compile. The gap between "the model wrote a file" and
  "a person can see it" is one `gjs` invocation.
- **The verify loop needs no toolchain**, so `lisa check` can run on a
  reference iMac, in CI, and on a macOS dev host for everything that does
  not need `gi://` — the split `shell/testing/README.md:19-22` already
  requires of app code.
- **#48 leaves the app-authoring critical path.** Issue #48 pins
  clang/cmake/ninja/pkgconf into `/var` because `flutter build linux`
  needs them. ADR-0047 parked Flutter, so that payload now serves only the
  parked lane. #48 should be **re-scoped to say so**, not closed by this
  ADR — it is still the correct decision for the lane it describes.
- **A related consequence, stated because it contradicts a live rule.**
  `lisa forge --setup` fetches a ~1 GB Flutter SDK to `/var/lib/lisa/flutter`
  and its own error text says *"run `sudo lisa forge --setup`"*
  (`cli/lisa/src/main.rs:1264-1267`). That is a user-facing `sudo` path,
  which CLAUDE.md 7b and ADR-0034 §3 forbid. Under ADR-0047 it is on the
  parked lane rather than the app road, so this ADR does not remove it —
  it records that the GJS default is what takes the last `sudo` off the
  path to writing an app.

## Where the guardrail analogy stops

The strongest version of this argument is: *§7 is a prompt, and prompts
are probabilistic; a scaffold is a mechanism.* That is right in direction
and must not be overstated, so here is the boundary, tested against
ADR-0030's own invariant.

**ADR-0030 §2:** *"The boundary must not be reachable from inside."*

- **A scaffold fails that test.** The model can edit or delete every line
  it generated. A scaffold constrains the *start* of a file, never its
  later edits. It is **not a guardrail in the security sense**, and
  nothing in Lisa's threat model should ever depend on one.
- **A scaffold is not even aimed at the model.** ADR-0030's second test —
  *is it aimed at the model or at the owner?* — returns neither. It is
  aimed at the blank page. Guardrails sit between the model and the
  machine; a scaffold sits between an author and an empty directory.
- **The check is the part with guardrail *shape*, and only inside the
  loop.** `Verifier::check` is invoked by the harness after the model's
  turn, and a `DoneClaimed` is checked rather than believed
  (`libs/forge-harness/src/agent.rs`). For the duration of a forge run the
  model cannot mark its own work passing. That is real, and it is also
  narrow: a human editing the file afterwards is outside it by
  construction, and the thing that covers *them* is CI — which today does
  not run app tests at all (`ANATOMY §5`: `.github/workflows/ci.yml:154-170`
  globs only `shell/*/tests/`).

So the honest claim, and the one this ADR makes:

> Scaffolding and checking move a rule from **prose a reader may follow**
> to **code that starts correct** and **a command that fails**. That is
> the difference between #241 — which shipped, ran, drew a window and
> silently had no capability — and a defect that cannot converge. It is
> not a security boundary, and calling it one would cheapen the term
> ADR-0029/0030 spent three review rounds earning.

## Where `lisa forge`'s analysis falls short of the traps

Assessed against the five traps and the entry-module footgun. `dart
analyze` is a Dart type and syntax checker; **zero of the six is covered
by any verifier that exists**, and for GJS there is no verifier at all.
What a checker can honestly claim:

| trap | mechanically decidable? | the honest form |
|---|---|---|
| **#241** manifest destination | **yes** — a build-tree check: the app's manifest `install` line must target `/usr/share/lisa/manifests`. ADR-0049's first slice asks for exactly this | a `lisa check` gate + the `cargo test` shape of `apps_payload.rs` |
| **top-level `await`** in the entry module | **yes**, and the highest value per line — the failure emits nothing to any log | a source check on the entry module |
| **#218** `tools[name]` | **partly**. The literal pattern is greppable; a differently-spelled equivalent is not. The durable fix is one shared module (ADR-0047 §6.1) | scaffold the guard *and* the mutation-checked test; the grep is interim |
| **#219** socket lifecycle | **no** — proving a socket is released needs a process. A text check that all three connections are present is weak and should say so | scaffold all three; check presence, claim nothing more |
| **#212** agent world isolation | **no, and not general** — WebKit-specific, one app. A `world_name != null` check is a Surfer rule, not an app rule | out of the minimal set |
| **#210 / #223** silent-null fallback | **no.** `?? ''` is legitimate constantly; the defect is semantic. This one stays a review rule and the fixture-honesty discipline (`ANATOMY §5`) | not claimed by the checker |

Two of six fully, one partly, three not at all. A checker built on that
honest ledger is worth having; a checker described as covering "the
traps" would be the same defect the traps are made of.

## What would reverse this

- **A compiled or second toolkit returns.** A build step makes `build`
  and `package` verbs real work rather than ceremony, and the "no
  toolchain" argument for one-shot generation collapses with it.
- **Per-app packaging lands** (ADR-0048 §5, ADR-0046). Then `lisa package`
  has a consumer and should exist. This is deferral, not rejection.
- **The scaffold turns out not to be needed** — a harness with the
  ANATOMY document in context one-shots a clean app at a rate we would
  accept anyway. That is measurable, and nobody has measured it. If true,
  keep `lisa check` and drop `lisa new`; the checker carries the argument,
  the generator only carries the convenience.
- **`libs/lisa_ui` absorbs the Agent Bus edge** (ADR-0047 §6.1). Then the
  scaffold *imports* rather than generates the #218 and #219 code, and
  those checks move to one library with one corpus entry. That shrinks
  this ADR rather than reversing it, and is the outcome to aim for.
- **A GJS static analyzer we trust appears.** `lisa check` should call it
  rather than grow its own opinions about JavaScript.

## Consequences

- **We take on a checker to maintain**, and a wrong check is worse than no
  check: it manufactures confidence. Every rule `lisa check` enforces must
  cite the issue that produced it, the way `libs/lisa-guard`'s corpus
  does — a check with no incident behind it is somebody's taste with an
  exit code.
- **The checker is a ratchet, not a proof.** It only ever contains what
  someone wrote down after a bug shipped. That is exactly its value and
  exactly its ceiling.
- **`ANATOMY §7` keeps its narrative and loses its checklist.** The
  stories are why the rules are believed; the imperative list becomes
  `lisa check`.
- **The Forge gets a verifier for the toolkit ADR-0047 chose**, which it
  does not have. Until then `lisa forge` produces Flutter or nothing.
- **CI is the missing half.** App tests are wired into `just shell-test`
  and enforced by nothing automated. `lisa check` in CI over `apps/*` is
  what makes any of this hold for code a person edits after the loop ends.
