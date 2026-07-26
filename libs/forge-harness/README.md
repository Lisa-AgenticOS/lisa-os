# forge-harness — the agentic app-building loop

Spec: docs/PLAN.md §5.12.1. Milestone: M6. Governance: ADR-0004 (Flutter
lane), ADR-0027 (on-device SDK + launch).

plan → edit files (jailed to project dir) → `dart analyze` / `flutter
analyze` → iterate. Pluggable backends: local coder models, a remote
provider, or any agent CLI over the same tool jail. Hot-reload preview and
VLM screenshot self-inspection are still ahead.

Driven from the CLI (`cli/lisa`, `lisa forge`):

| verb | what it does |
|---|---|
| `lisa forge --setup` | install the pinned Flutter SDK to `/var/lib/lisa/flutter` — sha256-pinned tarball on x86_64, commit-pinned checkout on aarch64 (ADR-0027) |
| `lisa forge --flutter "…"` | scaffold a lisa_ui app (pubspec + `LisaApp` stub + smoke test) and run the loop with `flutter analyze` as the verifier |
| `lisa forge --build` / `--run` | `flutter build linux --release`, install the bundle under the forge apps dir, write the `.desktop` entry, optionally launch |

The workflow itself is a skill (`skills/build-lisa-ui-app/SKILL.md`,
ADR-0025), not hardcoded prose.

Status: **loop live** — plan→edit(jailed)→analyze→iterate converges
against real models and the scripted-backend test; the Flutter lane
scaffolds, verifies, builds and installs.
