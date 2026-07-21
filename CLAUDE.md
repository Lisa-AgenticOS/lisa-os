# CLAUDE.md — working agreements for the Lisa OS monorepo

Lisa OS is an AI-native Linux distribution: local models as a system
service, per-app durable context, MCP-native agent surfaces, and an
append-only audit Ledger. **`docs/PLAN.md` is the source of truth** for
architecture and scope; this file is the operating manual.

## Commands

| Task | Command |
|---|---|
| Build everything | `just build` (cargo build --workspace) |
| Run all tests | `just test` |
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
| `shell/*` | PLAN §5.7 | M4 |
| `apps/*` | PLAN §5.8 | M6 |
| `libs/lisa_ui`, `libs/lisa_flutter`, `libs/forge-harness`, `forge/` | PLAN §5.12 | M6 |
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
   `lisa-agentd` never get network access; only `lisa-modeld` (model
   traffic) does. Never add a network dependency to a no-egress daemon.
6. **Provenance is load-bearing.** Context chunks carry provenance tags;
   untrusted provenance never triggers privileged tool calls without
   escalated confirmation (PLAN §5.10, Appendix C).
7. **One command center.** User-facing CLI verbs live under `lisa <verb>`
   — no scattered `lisa-*` helper scripts (Appendix E, rule 4).
8. **No invented external references.** Model sources, URLs, and hashes
   are pinned to verified artifacts or left explicitly unset — never
   guessed (see `models/catalog/catalog.toml`).
9. **Commits:** imperative mood, reference the PLAN section or ADR when
   relevant. No AI co-author/attribution lines.

## Repo mechanics

- Cargo workspace members: `libs/liblisa`, `daemons/inferenced`,
  `daemons/modeld`, `cli/lisa`. New Rust components join the workspace.
- Non-Rust components (mkosi profiles, GNOME Shell extension, Flutter
  lane) keep their own toolchains; the `justfile` is the umbrella.
- Dev host may be macOS: everything in the Rust workspace must build and
  test on macOS *and* Linux; `just image`/`just vm` and systemd/portal
  work are Linux-only and run in CI.
- Track L (pacman layer on stock Arch/Omarchy) ships from `os/layer/`;
  Track I (immutable mkosi image) from `os/mkosi/`. Track L is the
  distribution channel while Track I matures (ADR-0003).

- `git config core.hooksPath .githooks` once per clone: the pre-push
  hook runs the lint gate so an unverified push cannot slip out.
