# CLAUDE.md — working agreements for the Lisa OS monorepo

Lisa OS is an AI-native Linux distribution: local models as a system
service, per-app durable context, MCP-native agent surfaces, and an
append-only audit Ledger. **`docs/PLAN.md` is the source of truth** for
architecture and scope; this file is the operating manual. **`docs/STATUS.md`**
is the living "where are we" snapshot — read it first to catch up.

## Commands

| Task | Command |
|---|---|
| Build everything | `just build` (cargo build --workspace) |
| Run all tests | `just test` |
| Shell/IME unit tests | `just shell-test`, `just ime-test` (any dev host) |
| Lint (CI gate) | `just lint` (fmt --check + clippy -D warnings) |
| Format | `just fmt` |
| Local smoke test | `just smoke` (daemon + `lisa ask` round-trip) |
| OS image (Linux/CI only) | `just image`, `just vm` |

Run `just lint && just test` before every commit; CI enforces both.

## Component map

| Path | Spec | Milestone |
|---|---|---|
| `daemons/inferenced` | PLAN §5.1 | M1 |
| `daemons/modeld` | PLAN §5.2 | M1 |
| `daemons/contextd` | PLAN §5.3 | M3 |
| `daemons/agentd` | PLAN §5.4 | M5 |
| `portals/xdg-desktop-portal-lisa` | PLAN §5.5 | M2 |
| `libs/liblisa` (+ gtk/qt) | PLAN §5.6 | M2 |
| `shell/*` — **also extracted to the `lisa-desktop` repo (ADR-0039); duplicated here pending step 6** | PLAN §5.7, ADR-0038, ADR-0048 | M4 |
| `apps/*` — **also extracted to `lisa-apps` (ADR-0039)**; incl. `apps/files`, `apps/photos` — not started | PLAN §5.8, ADR-0048 | M6 |
| `libs/forge-harness`, `forge/` | PLAN §5.12, ADR-0047 | M6 |
| `libs/lisa_ui` — **still the parked Dart kit; the GJS shared library ADR-0047 §6 asks for is UNBUILT. Do not import this to build an app** | ADR-0047 §6 | — |
| `libs/lisa_flutter` | parked (ADR-0047) | — |
| `ime/fcitx5-lisa` | PLAN §5.7.3 | M4 |
| `cli/lisa` | PLAN §5.4 (scriptability) | M1+ |
| `os/*` | PLAN §3, §6 | M0+ |
| `models/*` | PLAN §7 | M1 |
| `tests/*` | PLAN §11 | per suite |

## Rules

1. **Read the spec first.** Before touching a component, read its §5.x
   block in `docs/PLAN.md`. Component READMEs mirror their spec; keep them
   in sync when behavior changes.
2. **Acceptance-block discipline.** A milestone is done only when its
   Acceptance block passes in CI. Anything not in an Acceptance block is
   backlog, not scope.
3. **ADRs over silent improvisation.** When the plan conflicts with
   reality (dead library, changed API, superseded model), write
   `docs/adr/NNNN-slug.md` and proceed with the substitute. The model
   catalog (`models/catalog/`) is *data, not law*.
4. **Boring tech for plumbing.** systemd, D-Bus, SQLite. Rust for daemons
   and SDK core; TypeScript/GJS for Shell surfaces; Python only for build
   tooling and evals. Shell script only in installers and hooks — never as
   substrate.
5. **Egress is architecture.** `lisa-inferenced`, `lisa-contextd`, and
   `lisa-agentd` never get network access; **only `lisa-remoted` does**
   — it is the sole egress broker (ADR-0010, PLAN §4 dataflow rule 2).
   Never add a network dependency to a no-egress daemon.
   *(This rule said `lisa-modeld` until 2026-08-05. It was wrong:
   `modeld` is the content-addressed model store and ships no unit at
   all, while `remoted` is the one door out. PLAN §5.10 and VISION had
   both already carried the correction in-line — the operating manual
   was the last copy still naming the wrong daemon, which is the
   sharpest possible argument for one source of truth.)*
6. **Provenance is load-bearing.** Context chunks carry provenance tags;
   untrusted provenance never triggers privileged tool calls without
   escalated confirmation (PLAN §5.10, Appendix C).
6a. **Probabilistic inside, logical outside** (ADR-0029, ADR-0030). Agent
   safety is deterministic code the model cannot reach, never prompt
   text. Two tests before shipping any guardrail: *is it reachable from
   inside?* (if yes it is not a guardrail) and *is it aimed at the model
   or at the owner?* (guardrails sit between the model and the machine,
   never between a person and their own machine). New rule ids join the
   catalogue in `cli/lisa/src/guard.rs` and a corpus entry in
   `libs/lisa-guard/tests/corpus.rs` — a rule with no corpus entry is one
   nobody will notice regressing.
6b. **Identity comes from the transport** (ADR-0033). Never trust an
   `actor`, `app_id`, scope list or provenance chain because the message
   says so. Ownership = `lisa_peer::PeerId`/`Owner` (the broker-assigned
   unique name); program identity = peer credentials and
   `/proc/<pid>/exe`, never `comm`. Anything a *later* call can act on —
   a parked confirmation, a session, a namespace — stores its `Owner` and
   checks it. A refusal must not reveal what exists.
7. **One command center.** User-facing CLI verbs live under `lisa <verb>`
   — no scattered `lisa-*` helper scripts (Appendix E, rule 4).
7a. **The install, update and recovery paths may not depend on
   infrastructure we do not control** (ADR-0034). Everything else may.
   Issue #45 is why: `lisa update` was one upstream reshuffle away from
   being unable to download, because libcurl arrived only as an
   accidental transitive dependency.
7b. **`/var` is the system's, `$HOME` is the user's** (ADR-0034).
   System-scope payloads — models, the app channel, the runtime channel —
   live on `/var`. Per-user tooling lives in the user's home, which is a
   real partition (ADR-0019) and therefore already survives A/B updates.
   Nothing user-facing should need `sudo`; `escalate.privilege` is an
   unoverridable `Deny` in our own guard.
8. **No invented external references.** Model sources, URLs, and hashes
   are pinned to verified artifacts or left explicitly unset — never
   guessed (see `models/catalog/catalog.toml`).
9. **Commits:** imperative mood, reference the PLAN section or ADR when
   relevant. No AI co-author/attribution lines.
10. **Everything we build is documented, and only what exists.** Every
   component directory carries a `README.md` answering four questions:
   *what it does*, *how it works* (with the smallest real usage example),
   *how to extend it*, and *its limits* — including known-broken things
   with issue numbers. A decision gets an ADR; a shipped feature gets
   user-facing docs. **Never write a user guide for something unbuilt** —
   `tests/acl-fuzz` was a README describing a suite that did not exist,
   and `acl.rs` told readers it ran. Documenting intent as if it were
   behaviour is the single most repeated defect in this repo's history.
11. **We write the apps; we do not patch GNOME's** (ADR-0048). Lisa
   Desktop is a desktop of our own — the Shell is forked (ADR-0038), the
   first-party apps are GJS/GTK4 and MCP-native (ADR-0047), and
   **GTK4/libadwaita and Mutter are never forked**: toolkit and
   compositor are foundation, not experience. Where a Lisa app does not
   exist, ship the stock GNOME app *unpatched* — that is the honest
   interim, not a gap to close with a patch set. Divergence stays narrow
   and deliberate (input, the prompt surfaces, the dock, agent
   affordances); rebase cost scales with the width of the delta.

## Repo mechanics

- **Four repos, and this one is not the only source (ADR-0039).**
  `lisa-desktop` (shell surfaces + IME, and the vendored GNOME Shell
  fork), `lisa-apps` (Mail, Surfer, Preview) and `lisa-packages` (the
  signed `[lisa]` index) were extracted on 2026-08-02 with history.
  **Nothing was deleted here**, so `shell/*` and `apps/*` exist in both
  places — step 6 (removal) is untouched, and ADR-0039's own failure
  clause has therefore triggered. Before editing `shell/` or `apps/`,
  know which tree ships the thing you are changing: the image installs
  `lisa-desktop-shell` from the hosted index (built by `lisa-desktop`),
  while the GJS surfaces still ship from this repo's own packaging.
  See `docs/VISION.md` for the four-repo table and #171 for the
  remaining steps.
- **Fork packages replace stock by contract, never by name.** A fork of
  a GNOME component is `lisa-desktop-<thing>` carrying
  `provides=`/`conflicts=` on the stock name — not a package that takes
  the stock name and outranks it by `pkgrel`. That race silently loses
  the day Arch ships a higher version, which it did on 2026-08-04.
- Cargo workspace members: `libs/liblisa`, `daemons/inferenced`,
  `daemons/modeld`, `cli/lisa`. New Rust components join the workspace.
- Non-Rust components (mkosi profiles, the GJS shell surfaces and apps,
  the parked Flutter lane) keep their own toolchains; the `justfile` is
  the umbrella.
- Dev host may be macOS: everything in the Rust workspace must build and
  test on macOS *and* Linux; `just image`/`just vm` and systemd/portal
  work are Linux-only and run in CI.
- Track L (pacman layer on stock Arch/Omarchy) ships from `os/layer/`;
  Track I (immutable mkosi image) from `os/mkosi/`. Track L is the
  distribution channel while Track I matures (ADR-0003).

- `git config core.hooksPath .githooks` once per clone: the pre-push
  hook runs the lint gate so an unverified push cannot slip out.
