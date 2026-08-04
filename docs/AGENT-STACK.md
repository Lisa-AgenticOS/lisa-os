# The agent stack

Nine ADRs describe pieces of Lisa's agent system and none of them
describes the whole. This is the map: what each process is, which one
holds the model, which one holds policy, which one holds the human, and
what a request actually touches on its way through.

**Governing decisions:** ADR-0025 (one agent loop), ADR-0029 / ADR-0030
(hard guardrails and where the boundary sits), ADR-0033 (identity comes
from the transport), ADR-0036 (an assistant that acts on its own),
ADR-0043 / ADR-0044 (retrieval and receipts), ADR-0049 (every app is an
agent surface). **Spec:** `docs/PLAN.md` §5.1–§5.11, Appendix B (the
manifest), Appendix C (the provenance envelope).

Every claim below cites a file, and where a mechanism is *decided but
not built* it says so and names the issue — see
[Decided, not built](#decided-not-built) and
[Where the code disagrees with the ADRs](#where-the-code-disagrees-with-the-adrs).
Companion documents: `docs/ANATOMY-OF-AN-APP.md` (what an app is, from
the app author's side) and `docs/PLAN.md` (the spec).

Verified against the tree on **2026-08-04**.

---

## 1. The shape in one picture

```
  a person                                        a person
     │                                               ▲
     ▼                                               │ (confirm / deny)
┌──────────────┐   Harness1.Run   ┌──────────────┐   │
│ shell/       │─────────────────▶│ lisa-        │   │
│  assistant   │◀── Token/Tool ───│  harnessd    │   │
└──────────────┘     /Finished    │ HOLDS THE    │   │
                                  │ MODEL        │   │
  cli/lisa ────── in-process ────▶│ (loop)       │   │
  (`lisa assist`)   same loop     └──────┬───────┘   │
                                         │           │
             /v1/chat/completions         │  Agent1.RequestCall
                    ┌────────────────────┘│                │
                    ▼                      ▼               │
            ┌──────────────┐        ┌──────────────┐        │
            │ lisa-        │        │ lisa-agentd  │────────┘
            │  inferenced  │        │ OWNS POLICY  │  ConfirmationRequested
            │ no egress    │        │ no egress    │
            └──────┬───────┘        └──────┬───────┘   ┌──────────────┐
                   │ unix socket           │ MCP       │ lisa-        │
                   ▼                       ▼           │  consentd    │
            ┌──────────────┐   /run/user/<uid>/lisa/mcp/│ DRAWS THE   │
            │ lisa-remoted │        app.lisaos.*.sock   │ DIALOG      │
            │ THE ONLY     │               │            └──────────────┘
            │ EGRESS       │               ▼
            └──────┬───────┘        ┌──────────────┐
                   │                │ apps/*       │
                   ▼                │ MCP servers  │
              the internet          └──────────────┘

  lisa-modeld ── the other egress (model weights only)
  lisa-contextd ── the index, embeddings, provenance; no egress
```

Three processes, three jobs, and **none of them able to approve its own
work** — `daemons/harnessd/src/main.rs:10-25` states the split and why:

- **harnessd** hosts the model and runs the loop.
- **agentd** owns policy: tiers, provenance, the Ledger, the undo
  journal. It never sees a model.
- **lisa-consentd** raises the human's dialog and nothing else.

Before issue #145, the process hosting the model also owned the consent
surface, so a call it originated came back to `Confirm` from the same
peer that asked and the model approved itself.

---

## 2. The processes

### `daemons/inferenced` — the model runtime

OpenAI-compatible HTTP on loopback plus `dev.lisaos.Inference1` on the
session bus (`daemons/inferenced/src/api.rs:1-4`,
`daemons/inferenced/src/dbus.rs:42-196`). No network of its own
(CLAUDE.md rule 5).

**Two lanes, and the split is load-bearing.**
`daemons/inferenced/src/api.rs:404-419` routes on a *non-empty* `tools`
array:

- the **typed lane** — guided generation (JSON Schema → GBNF), the
  scheduler's priority classes, token streaming;
- the **tools lane** — `raw_chat` / `raw_chat_stream`
  (`daemons/inferenced/src/engine.rs:47-76`), the request body passed
  through **verbatim**, because a tool turn carries null content and
  roles the typed `ChatMessage` cannot represent.

OpenAI SDKs send `"tools": null` or `[]` on plain requests, so an empty
array deliberately stays on the typed lane (#35).

`Content` is `Text(String)` or `Parts(Vec<Value>)`
(`daemons/inferenced/src/openai.rs:41-44`), and the parts stay
`serde_json::Value` on purpose: Lisa passes provider part schemas
through rather than re-modelling them, because re-modelling means a
silent drop for a modality nobody modelled — *and a dropped image still
gets a confident answer about an image nobody saw.* A text-only engine
refuses rather than guessing, with one shared sentence
(`openai.rs:81`, `TEXT_ONLY_REFUSAL`). #236 is why that sentence is a
constant: the refusal lived only in the typed lane's `open_stream`, and
the tools lane — the one behind every Assistant window — had no copy of
the rule *or* the text, so nothing about its absence was visible
(`daemons/inferenced/src/llama.rs:490-524`).

Remote models are a name prefix: `remote:<provider>:<model>` is
forwarded to lisa-remoted over a unix socket
(`daemons/inferenced/src/remote.rs:1-15`, `REMOTE_PREFIX` at `:26`).
inferenced gains no network in the process.

### `daemons/modeld` — weights, and the other egress

The model catalog is *signed data, not code*
(`daemons/modeld/src/catalog.rs:1-3`) — a TOML index of models, licences
and hardware requirements, sourced from `models/catalog/catalog.toml`.
`fetch.rs`, `store.rs`, `profile.rs` and `recommend.rs` download,
verify, place and choose. This and remoted are the only two components
with network access.

### `daemons/remoted` — the egress broker

`daemons/remoted/src/lib.rs:1-6`: the only component besides modeld with
network access. Every remote request is ledgered with the `remote.`
"leaves your hardware" marking **before** egress, and per-scope offload
consent defaults to *nothing leaves*.

The scopes are `prompt`, `files`, `mail`, `calendar`, `screen`, `memory`
(`daemons/remoted/src/consent.rs:14`) — the user's own prompt text does
not leave the device until explicitly enabled. A request declares the
scopes it carries and **any** scope not switched on refuses the whole
request (`consent.rs:1-5`).

### `daemons/contextd` — index, embeddings, provenance

Per-user SQLite: FTS5 lexical index, per-app memory namespaces, file
ingestion with provenance tags, scoped retrieval, every retrieval
ledgered (`daemons/contextd/src/lib.rs:1-17`). No egress.

Provenance is a closed set — `file`, `mail`, `calendar`, `screen`,
`web`, `system` (`daemons/contextd/src/acl.rs:22`) — and a write with
anything else is **refused** (#104). It used to be accepted and then
silently unreadable, so a plugin with a typo produced an invisible index
and nothing distinguished "indexed nothing" from "indexed into a tag no
scope can reach".

The ACL filters **at the query**, mapping granted portal scopes to
allowed provenance, so a disallowed chunk cannot leak through ranking
(`acl.rs:1-6`). An app granted `documents.read` never receives a `mail`
chunk even when it is the best hit.

Embeddings are honest about themselves: `InferencedEmbedder` when
inferenced's socket answers, `HashEmbedder` otherwise — and the fallback
logs a warning, prints a CLI note, and writes `"embedder": "hash"` into
the Ledger entry (`daemons/contextd/src/embed.rs:7-14`). That is the
whole of #163: `hybrid=true` used to return plausibly-ranked hits with
no semantic model behind them and nobody could tell.

### `daemons/agentd` — the Agent Bus, and the only policy engine

The system MCP host: registry of installed servers, discovery, and
execution under bus-enforced tiers (`daemons/agentd/src/lib.rs:1-32`).
Never gets network access.

**Manifests are loaded once, by a directory walk at daemon start**
(`daemons/agentd/src/main.rs:70-86`). `SYSTEM_MANIFEST_DIR` is
`/usr/share/lisa/manifests` (`main.rs:17`), always searched first,
followed by `LISA_MANIFEST_DIRS` (which may only *append* — #134) and
then `$XDG_DATA_HOME/lisa/manifests`. The **first** definition of an
`app_id` wins (#97), so a user-writable manifest may add an app but
never redefine a system one, and a clash is reported rather than silent
(`daemons/agentd/src/registry.rs:106`).

The call state machine is `daemons/agentd/src/bus.rs`: request → tier
resolution with provenance escalation → silent execute *or* park for
confirmation → confirm/deny → execute. Its four invariants
(`bus.rs:1-15`) are the load-bearing ones in the whole stack:

- **No ledger entry, no action.** The `tool.call` entry is appended
  before dispatch; an unavailable Ledger means the call never runs.
- **No unconfirmed privileged calls.** Only a `read`-tier tool with a
  fully trusted trigger chain executes silently.
- **Every executed privileged call is journaled** with its resolved
  compensation, or an explicit "not undoable".
- Parked confirmations expire — `CONFIRMATION_TTL` is 120s
  (`bus.rs:35`), capped at `MAX_PENDING = 128` overall and
  `MAX_PENDING_PER_OWNER = 16` (`bus.rs:47-48`, #137). Denying the bus
  denies the confirmation *surface*, so the cap is a soft bypass of the
  second invariant — it makes confirmation unavailable rather than
  defeating it, and `bus.rs:38-46` says so out loud.

Tier policy is `daemons/agentd/src/tier.rs`: `Provenance` at `:118`,
`Confirmation` at `:282`, `resolve(declared, chain)` at `:383`. An
unrecognised provenance tag is untrusted by construction.

Every parked call stores its `Owner` (`bus.rs:269`, `lisa_peer::Owner`),
and only that owner may answer it (`bus.rs:565`). A wrong-owner answer
is made **indistinguishable from "no such call"** (`bus.rs:59-62`) so a
sweep cannot use the error to map which ids exist.

### `daemons/harnessd` — the one agent loop, as a service

`dev.lisaos.Harness1`: `Run` plus `Tool` / `Token` / `Finished` signals
and `Cancel` (`daemons/harnessd/src/dbus.rs:14-16`, `:283-452`). Every
surface that wants an assistant drives this rather than growing a loop
of its own — two loops would mean two answers to "which tools may the
model use" (`main.rs:1-8`).

**The trust ceiling comes from the caller, never the message** (commit
`4957782`, #229/#230/#231/#236/#215). `Run` takes a `trigger` option and
it is not believed: `caller::ceiling` (`caller.rs:100-106`) returns
`Trigger::Prompt` only when the broker says the caller is this user
**and** currently owns a name in `PROMPT_SURFACES` — one entry today,
`app.lisaos.Assistant` (`caller.rs:63`). Everything else, including a
caller that could not be placed at all, is `Trigger::Event`, whose
content is never trusted. `Trigger::resolve` takes the *lower* of
requested and ceiling (`dbus.rs:92`), so a caller may narrow itself but
never widen.

`facts_of` fails towards `CallerFacts::UNKNOWN` (`caller.rs:113-118`):
a broker that will not answer is a caller we cannot place, and the safe
reading of "cannot place" is "not a person".

The identity mechanism here is deliberately *not* the one used
everywhere else. `caller.rs:15-43` explains: `lisa-harnessd.service` is
a per-user unit carrying `ProtectHome` / `ProtectSystem=strict` /
`PrivateDevices`, a user manager delivers those through an implicit user
namespace, and from inside one `readlink /proc/<peer>/exe` returns
EACCES for every caller (#161). So the ceiling is built from
`GetConnectionCredentials` and `GetNameOwner` — both broker-assigned and
unforgeable by the sender — and the file says so out loud so the next
reader does not "fix" it into an exe check that always refuses. Its own
honest limit is stated at `caller.rs:44-50`: a well-known name belongs
to whoever asks first, so a peer can inherit the Assistant's ceiling if
the Assistant is not running.

The trigger class also decides whether the file tool family **exists**:
`dbus.rs:334-340` filters the workspace to `None` when
`trigger.may_use_file_tools()` is false, applied to the workspace rather
than to the provider list so the tools and the system prompt cannot
disagree — *strip the tools without stripping the sentence that promises
them and the model confidently claims to have saved something.*

---

## 3. The libraries

### `libs/bus-tools` — the Read filter

`read_tier_tools` (`libs/bus-tools/src/lib.rs:61`) turns agentd's
`ListTools` JSON into the tools the model is offered. It is documented
as *the only thing keeping write-tier tools away from the model*
(`lib.rs:38-44`): `navigate`, `click`, `fill`, `create_note`,
`archive_message` are all registered and all reachable by anything that
can open the socket. **A row with no tier is dropped, not defaulted to
read** — "defaulting the unknown to the permissive value is how a
fail-open lands in a security boundary" — and one malformed manifest is
skipped rather than costing every other app its tools.

`wire_name` (`:280`) flattens `app.lisaos.notes::create_note` into an
OpenAI-legal `[A-Za-z0-9_-]{1,64}` name.

`outcome_for` (`:100`) is the other half of the split: a `confirm-chip`
or `confirm-modal` disposition becomes a tool **failure** the model is
told about — *"needs a person to confirm it and none is present; the
call is parked, not done. Do not retry it."* — because calling `Confirm`
from here would make the model both requester and approver
(`lib.rs:91-99`).

`result_is_web_tagged` (`:124`) **parses** the result JSON rather than
searching it, so a page that merely contains the string
`"provenance":"web"` cannot taint the chain by mention.

### `libs/harness-core` — sessions, memory, skills, turns

A plain sync library: no HTTP, no D-Bus, no daemons
(`libs/harness-core/src/lib.rs:1-9`). `Session` (multi-turn
conversations on the context fabric), `Memory` (durable per-scope notes
plus the bounded digest injected each turn), `Skill`, and `Turn` (pure
composition of one request body).

`Skill` is a SKILL.md file with hand-parsed `key: value` frontmatter —
`name` required, `description` and `tools` optional
(`libs/harness-core/src/skill.rs:218-266`); unknown keys are ignored for
forward compatibility. Only the one-line `catalog_line()` goes in a
prompt; `body()` reads from disk on use. `resolve()` (`:191`) routes a
prompt to a skill by deterministic token overlap, requiring a genuine
*name*-token hit — several description hits can sum past any threshold.

`Skill::allowed_by` (`:136`) **intersects** across active skills:

> A skill that declares `tools: [read_file]` is saying what it needs,
> and a second skill being active is not a reason to widen the first
> one — union semantics would mean activating any unrestricted skill
> silently restores the full tool set, which is an allowlist that
> anything can switch off.

### `libs/forge-harness` — the loop itself

`forge_agent_observed` converses with the backend one tool call at a
time. Tool families are `ToolProvider`s merged by `dispatch`
(`libs/forge-harness/src/agent.rs:396-404`) — first provider to claim a
name wins, so **the caller's ordering is its precedence**.

The built-in workspace family is `read_file`, `list_dir`, `grep`,
`write_file`, `edit_file`, `run_command`, `run_tests`
(`libs/forge-harness/src/tools.rs:110-201`), every file operation
mediated by the `Jail` and every command by `lisa-guard`. `run_shell`
(`libs/forge-harness/src/shell_tool.rs:93-110`) is the one escape hatch,
and its four conditions are structural rather than prompt-written
(`shell_tool.rs:11-34`): jailed, guard-checked, **never silent**, and
never unattended — `ShellTool::new` requires a consent callback and
there is no other constructor.

`AgentConfig.ledger` is not an `Option` and there is no `Default`
(`agent.rs:200-213`): "no ledger entry, no action" as something a caller
could forget is not an invariant. The ledger entry is appended *before*
the tool runs (`agent.rs:565-576`) so a crash mid-write still leaves
evidence of what was attempted.

The skill allowlist is enforced at `agent.rs:578`, and a refusal is
returned as tool **output**, not as an error that ends the run — which
is what makes an allowlist a constraint rather than a wall.

### `libs/lisa-guard` + `cli/lisa/src/guard.rs` — the deterministic guard

`libs/lisa-guard/src/lib.rs:1-24`: nothing in the crate consults a
model, reads a prompt, or depends on the model cooperating. `contain` is
the filesystem boundary; `check_command` (argv, no shell) and
`check_shell_line` (free-form) are the execution boundary. `Verdict` is
`Allow` / `Confirm` / `Deny`, and `Deny` is not overridable *from
inside*.

Two enforcement modes (`libs/lisa-guard/src/command.rs:170-187`):
`Enforced` restricts to `ALLOWED_COMMANDS`
(`command.rs:32-34` — `dart`, `flutter`, `cargo`, `ls`, `cat`, `grep`,
`find`, `echo`, `pwd`, `mkdir`, `touch`) for surfaces with nobody
watching; `Advisory` allows any program name with every catastrophic
rule still applied, for surfaces where a human reviews before anything
runs.

The rule catalogue with what relaxing each one costs is
`cli/lisa/src/guard.rs:16-76` — `escalate.privilege`, `rm.system_path`,
`rm.no_preserve_root`, `disk.raw_write`, `perm.system_path`,
`fs.system_write`, `audit.erase`, `command.not_allowlisted`,
`command.exec_predicate`, `command.unknown_subcommand`,
`command.denied_subcommand`, `command.path_escape`. `lisa guard allow`
refuses an unknown id, because relaxing a rule that does not exist would
look like it worked (`guard.rs:117-122`), and `lisa guard list` names
relaxations for ids this version does not know so stale policy cannot
accumulate (`guard.rs:98-109`).

The verb is the *outside* of the boundary (`guard.rs:1-7`): nothing the
agent can invoke reaches that file, which is why a hard refusal is safe
to make relaxable. Corpus: `libs/lisa-guard/tests/corpus.rs`, 599 lines
— CLAUDE.md rule 6a requires a new rule id to arrive with an entry
there.

### `libs/mcp-bus` — the transport

Newline-delimited JSON-RPC 2.0 over a per-app unix socket at
`<base_dir>/<app_id>.sock`, one short-lived connection per dispatch:
`initialize`, `notifications/initialized`, `tools/call`
(`libs/mcp-bus/src/lib.rs:1-10`). `McpDispatcher` mirrors agentd's
`bus::Dispatcher` signature exactly so it swaps in without touching the
state machine (`dispatcher.rs:1-5`), with a 10s per-operation timeout so
one hung app cannot wedge the bus (`dispatcher.rs:15`).

**Socket activation is deliberately deferred** — the app's socket must
already be live. That one fact is what makes socket presence *mean* tool
availability, and therefore what makes #219 (a socket outliving its app)
a correctness bug rather than untidiness.

### `libs/lisa-peer` — identity from the transport (ADR-0033)

`libs/lisa-peer/src/lib.rs:1-32`: five independent adversarial reviews
of agentd, the portal, remoted and contextd converged on one sentence —
*nothing in Lisa verifies who is calling* — and that is one missing
primitive absent twenty times, not twenty bugs.

Two mechanisms, kept separate on purpose:

- **`PeerId`** (`:75`) / **`Owner`** (`:154`) answer *"is this the same
  caller as before?"* — no `/proc`, no credentials round-trip. This is
  what binds a parked confirmation, a portal session or a memory
  namespace to whoever created it.
- **`Peer`** / `resolve` answer *"which user and process is this?"*,
  needed only where the decision depends on the program itself. From
  the broker, or from the kernel via `unix::peer_of_socket`.

Program identity is `/proc/<pid>/exe` through the broker's pidfd, never
`comm`.

### Sockets and search paths

| what | where |
|---|---|
| per-app MCP sockets | `$LISA_MCP_DIR` → `$XDG_RUNTIME_DIR/lisa/mcp` → `/run/lisa/mcp` (`daemons/agentd/src/main.rs:99-108`; the default constant is `libs/mcp-bus/src/lib.rs:19`) |
| manifests | `/usr/share/lisa/manifests` → `$LISA_MANIFEST_DIRS` → `$XDG_DATA_HOME/lisa/manifests` (`agentd/src/main.rs:17-31`) |
| skills (`lisa skills`) | `$LISA_SKILLS_DIR` → `$XDG_DATA_HOME/lisa/skills` → `/var/lib/lisa/apps/payloads/runtime/current/skills` → `/usr/share/lisa/skills` (`cli/lisa/src/skills.rs:20-38`) |
| skills (harnessd) | `$LISA_SKILLS_DIR` → `$HOME/.local/share/lisa/skills` → `/var/lib/lisa/apps/current/skills` → `/usr/share/lisa/skills` (`daemons/harnessd/src/skills.rs:18-33`) — **the third entry does not match the CLI's**, see below |

### `skills/` and `models/catalog/`

`skills/` is the repo's SKILL.md set, installed to `/usr/share/lisa/skills`
by `os/packages/lisa/PKGBUILD:302-305`. One skill ships today:
`skills/build-lisa-app`. Skills reach the model as a catalogue in the
system prompt plus a `read_skill` tool that fetches one body on demand
(`daemons/harnessd/src/skills.rs:75-116`) — the bodies are far too large
to keep resident, and an empty catalogue advertises no tool at all
rather than one that can only fail.

`models/catalog/catalog.toml` is the signed model index modeld parses
(`daemons/modeld/src/catalog.rs`). CLAUDE.md rule 8: sources and hashes
are pinned to verified artifacts or left explicitly unset.

### `cli/lisa` — the command surface

One command centre (CLAUDE.md rule 7). The agent-relevant verbs, and
what each one actually talks to:

| verb | path |
|---|---|
| `lisa ask` | inferenced's **typed** lane over HTTP; streaming, guided generation via `--json-schema`, priority via `--background` |
| `lisa do` | routes ONE utterance to exactly one bus tool and stops (`cli/lisa/src/agent.rs:174`); local models take the grammar router, `remote:` models the native tool-calling path, because remote providers expose tool calling but not sampler grammars |
| `lisa assist` | the multi-turn loop, in-process (`cli/lisa/src/bus_tools.rs:23`) — the tool family is shared with harnessd so every surface offers the same tools under the same rules |
| `lisa tools` / `lisa call` | agentd's `ListTools` / `RequestCall` directly (`agent.rs:233`, `:247`) |
| `lisa undo` | agentd's undo journal |
| `lisa guard` | the rule catalogue and relaxations — the *outside* of the boundary |
| `lisa skills` | `list` / `show` over the skill search path |
| `lisa context` / `lisa memory` | contextd: index, retrieve, remember, recall |
| `lisa ledger` | the audit record |

ADR-0050 decides that developer verbs live under **`lisa dev`** — `lisa
dev new` to scaffold an app and `lisa dev check` as the single authority
on what a valid app is. **No `lisa dev` verb exists**; the `Command` enum
(`cli/lisa/src/main.rs:33-350`) holds 27 verbs and `Dev` is not among
them. `lisa app` is reserved for desktop control, not authoring.

`lisa assist` refuses to start when agentd registered no read-tier tools
rather than running a loop with nothing to work with
(`bus_tools.rs:33-38`), and opens the Ledger before the loop for the
same reason forge does — *a machine with an unwritable Ledger refuses to
run rather than acting off it* (`bus_tools.rs:45-51`).

---

## 4. Three paths

### A chat turn (no tools)

1. The Assistant window calls `dev.lisaos.Harness1.Run`
   (`daemons/harnessd/src/dbus.rs:291`).
2. harnessd asks the broker who the caller is
   (`caller::facts_of`) and computes the ceiling; `Trigger::resolve`
   takes the lower of requested and ceiling (`dbus.rs:92`).
3. The workspace is filtered by `trigger.may_use_file_tools()`
   (`dbus.rs:340`); skills load and become a catalogue string.
4. `loop_runner::run` builds the system prompt from
   `harness_core::policy::policy_prompt()` + the assistant persona +
   whichever of the workspace/no-workspace paragraphs is true + the
   skill catalogue (`loop_runner.rs:133-152`). The prompt describes the
   **current grant**, not a fixed role.
5. The loop POSTs to inferenced on loopback. Tools present ⇒ the tools
   lane; tools absent ⇒ the typed lane.
6. `remote:`-prefixed models leave through remoted, which checks
   per-scope consent and ledgers `remote.` before egress.
7. Deltas come back as `Token` signals; the run ends with `Finished`.

### A read-tier tool call

8. The model emits a tool call with a wire name.
   `dispatch` (`agent.rs:399`) finds the owning provider.
9. The intent is appended to the Ledger **before** anything runs
   (`agent.rs:576`). A failed append aborts the run.
10. The active skills' allowlists are intersected (`agent.rs:578`); a
    refusal is returned as tool output.
11. `AgentBusTools::execute` calls `Agent1.RequestCall`
    (`libs/bus-tools/src/lib.rs:253`).
12. agentd resolves the tier: declared tier plus provenance escalation
    over the trigger chain (`tier.rs:383`). Read tier with a fully
    trusted chain ⇒ silent execute.
13. agentd ledgers the `tool.call` entry, then dispatches through
    `McpDispatcher` to `<socket_dir>/<app_id>.sock`.
14. The app answers with a provenance-tagged result. If the tag is
    `web`, `result_is_web_tagged` (`bus-tools:124`) parses it out and
    taints the chain for subsequent calls.

### A privileged call that parks

15. Same through step 12, except the resolution is `confirm-chip` or
    `confirm-modal`.
16. agentd stores the pending call with its `Owner` (`bus.rs:269`,
    `:513`) — subject to `MAX_PENDING` and `MAX_PENDING_PER_OWNER` — and
    emits `ConfirmationRequested`.
17. The loop is told the call is **parked, not done**, as a tool failure
    with an explicit "do not retry" (`bus-tools:100`). The model cannot
    call `Confirm`; that would make it requester and approver.
18. `shell/consent/lisa-consentd.js` draws the chip or modal. A person
    answers.
19. `Agent1.Confirm(id, bool)` arrives. `p.owner.allows(&answerer.peer)`
    (`bus.rs:565`) — anyone else gets an error deliberately
    indistinguishable from "no such call".
20. On yes: the call executes and its app-declared compensation is
    written to the undo journal, so `lisa undo` can revert it. On no, or
    after 120s: nothing happens and the entry is collected.

---

## 5. Where every gate sits

| gate | enforced in | fails |
|---|---|---|
| who is calling | `lisa_peer::resolve` / `PeerId` (broker-assigned) | closed — `CallerFacts::UNKNOWN` |
| what trust class a run may claim | `harnessd caller::ceiling` (`caller.rs:100`) | closed — `Trigger::Event` |
| do file tools exist for this run | `harnessd dbus.rs:340` | closed — no workspace, no family |
| which tools the model is offered | `bus_tools::read_tier_tools` (`lib.rs:61`) | closed — no tier, no tool |
| which tools an active skill permits | `Skill::allowed_by` (`skill.rs:136`), called at `agent.rs:578` | closed — intersection, never union |
| is this action recorded | `AgentConfig.ledger`, appended pre-dispatch (`agent.rs:576`) | closed — no entry, no action |
| does this call need a human | `agentd tier::resolve` (`tier.rs:383`) | closed — unknown provenance is untrusted |
| may this peer answer this confirmation | `bus.rs:565` (`Owner`) | closed, and silently — refusal reveals nothing |
| may this command run | `lisa_guard::check_command` / `check_shell_line` | `Deny` unreachable from inside |
| may this shell line run at all | `ShellTool` consent callback (`shell_tool.rs:11-34`) | closed — no constructor without one |
| may this context chunk be read | `contextd acl.rs` — filtered at the query | closed — unknown provenance refused at write |
| may this leave the machine | `remoted consent.rs:14` | closed — default is nothing leaves |

The pattern is CLAUDE.md rule 6a: each gate is deterministic code the
model cannot reach, and each one is aimed at the model rather than at
the owner. `lisa guard` is the deliberate exception — it sits on the
human's side of the boundary.

---

## Decided, not built

Do not write code that assumes any of this, and do not describe it to a
user as behaviour.

**The registry has no lifecycle (ADR-0049, #240 OPEN).** ADR-0049 §5
makes lisa-agentd the sole authority on what exists. `registry.rs` is
real — it loads, dedupes and ranks — but it is populated **once**, by a
directory walk at daemon start (`daemons/agentd/src/main.rs:70-86`).
Nothing registers at install, nothing deregisters at uninstall, nothing
re-scans. The evidence is visible on the reference device:
`app.lisaos.Browser` — renamed to Surfer months ago, no package, no
socket, no process — is still advertised to the model because
`~/.local/share/lisa/manifests/app.lisaos.Browser.json` was written once
and never reaped (ADR-0049:51-58).

**A manifest in a directory nothing reads is now a lint failure
(#241).** ADR-0049's first implementation slice asked for that check;
`os/repo-tools/check-app-manifests.py` is it, and `apps/preview`'s
manifest — which installed to `/usr/share/lisa/apps/`, the directory
that caught nobody's eye for months — now installs where agentd looks.
See `docs/ANATOMY-OF-AN-APP.md` §7.

**Installed-but-not-available is not a reported state (#219).**
ADR-0049's slice 2 closes the registry half of the socket-lifecycle
problem. Neither half exists; only Mail releases its socket on all three
exit paths.

**There is no install-time capability grant and no update comparison
(ADR-0049 §2, §4).** An update that widens capability should leave the
new tools inert until the person agrees. Neither exists. The shape it
will take is the portal's append-only grant log
(`portals/xdg-desktop-portal-lisa/src/grants.rs`), which is already in
the tree.

**Skills have no provenance and there are no per-app skills (ADR-0049
§1).** Skills are a system-wide search path, and `harness_core::Skill`
holds a **private** `path` with no accessor (`skill.rs:45`) — after
loading, nothing downstream can even ask where a skill came from.

**Socket activation is not implemented.** `mcp.activatable` parses; the
dispatcher never spawns (`libs/mcp-bus/src/dispatcher.rs:27-30`).

**`Trigger::Schedule` is unreachable by design** (`caller.rs:93-98`):
nothing in Lisa is a scheduler yet, and a ceiling handing out a class no
shipped peer can hold would be a hole with no user. ADR-0036's autonomous
triggers are a direction; the class exists, the source does not.

---

## Where the code disagrees with the ADRs

Found while writing this, each verified by reading the file.

**The skill `tools:` allowlist is inert in every shipping surface.**
`agent.rs:217-224` describes #57 as fixed — "a skill's `tools:`
frontmatter is an allowlist, and until now it was parsed and never
consulted" — and the enforcement at `agent.rs:578` is real. But
`AgentConfig.skills` defaults to empty (`agent.rs:272`) and **no
production caller ever sets it**: harnessd builds its config with
`..AgentConfig::new(ledger)` (`loop_runner.rs:161-172`) and hands skills
to the model as `read_skill` *text* instead; `lisa assist` does the same
(`cli/lisa/src/bus_tools.rs:53-57`). The only non-test population of the
field is `agent.rs:906`, a unit test. So today a skill's `tools:` line
is a declaration of intent that binds nothing outside `lisa forge`'s own
tests. The intersection semantics are correct; nothing calls them.

**The two skill search paths disagree.**
`daemons/harnessd/src/skills.rs:22` says "Mirrors `cli/lisa`'s
resolution", and it does not: harnessd's channel directory is
`/var/lib/lisa/apps/current/skills` (`:18`) while the CLI's is
`/var/lib/lisa/apps/payloads/runtime/current/skills`
(`cli/lisa/src/skills.rs:24`). `lisa skills list` and the loop's
catalogue can therefore see different sets after a runtime-channel
update. This is the same class of defect as #239, where the launcher's
private path list and the installer's had drifted one directory apart.

**Provider precedence is documented as a property and is actually a
convention.** `agent.rs:396-398` says "a later family can never silently
shadow the jail" — true only if the caller lists the jail first, and
harnessd lists the **bus first**, then the workspace, then skills
(`daemons/harnessd/src/dbus.rs:405-423`). It is safe today only because
`wire_name` (`bus-tools:280`) namespaces every bus tool as
`app_lisaos_<app>__<tool>`, so a collision with `write_file` cannot be
constructed from a manifest. The invariant holds by accident of naming,
not by the ordering the comment describes.

**The Forge cannot produce a Lisa app (#243).** ADR-0047 §4 says the
Forge targets GJS. `forge_cmd` writes a `pubspec.yaml` and selects
`Verifier::Dart` (`cli/lisa/src/main.rs:1559-1571`), and `Verifier` has
exactly three arms — `Dart`, `Command`, `None`
(`libs/forge-harness/src/agent.rs:108-112`) — with no GJS analyzer in
the tree, so a run reports "the project contains no Dart source files
yet" and can never converge on JavaScript. The surrounding machinery
agrees with the parked lane: `ALLOWED_COMMANDS`
(`libs/lisa-guard/src/command.rs:32-34`) lists `dart` and `flutter` and
has no `node`, `gjs` or `just`, and `run_tests`
(`libs/forge-harness/src/tools.rs:351-369`) recognises only
`pubspec.yaml` and `Cargo.toml`. The loop cannot run a GJS app's suite
by any route except `run_shell`, which asks a human every time.
ADR-0050 §3 names `lisa dev check` as the `Verifier::Command` arm that
closes this loop — **it does not exist**, and neither does any `lisa dev`
verb (`cli/lisa/src/main.rs:33-350` holds 27 verbs and `Dev` is not one).

**PLAN §5.4 and `agentd/src/lib.rs:25-29` still call the MCP wire
transport deferred.** `libs/mcp-bus` exists and `McpDispatcher` is
written; what remains deferred is socket *activation*. The doc comment
reads as though the whole transport is still a placeholder.
