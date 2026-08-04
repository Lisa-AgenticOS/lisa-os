# Forge — apps built where they run

Spec: docs/PLAN.md §5.12.1. Milestone: M6.

Split view: conversation left, live hot-reloaded preview right; template gallery; diff review; Install → sandboxed Flatpak with a user-approved capability manifest. Forged apps are born permissionless.

Status: **not started** — scaffold placeholder. Read the spec section (and CLAUDE.md rules) before writing code here.


## LisaCode — the app experience (the vision made concrete)

Apps on Lisa are not a fixed suite you're given — the OS ships the
**workshop**. "LisaCode" is the Forge: a Claude-Code-style harness,
native to the desktop, that takes *"make me a..."* to an installed,
sandboxed GJS/GTK4 app while you watch (PLAN §5.12.1, ADR-0047).
Everyone gets
their own apps by talking.

Loop (in `libs/forge-harness`, driven by `lisa forge`): plan → the model
writes a complete file → the **tool jail** confines it to the project
dir (absolute paths / traversal / placeholders are rejected and fed back
as fixable errors — the security boundary holds even against a bad model)
→ `lisa dev check` → findings feed back → repeat. The checker asserts
sources exist, that no entry module opens with a top-level `await`, and
that the manifest parses through *agentd's own* parser — it does not run
the app, because a verifier that executes model-authored code hands the
loop the escape the jail exists to prevent (#269).

**Pluggable backend (§5.12.1):** a local coder model (Qwen-coder at
Tier 2+), a **remote provider** (`--model remote:huggingface:...` for
quality on small machines), or a **BYO frontier agent** — Claude Code
itself slots in as a backend under the same jail. Verified live: the
loop runs end to end against the local model; convergence quality tracks
the model (a 1B model writes but doesn't converge; the scripted-backend
test proves convergence with correct code). Hot-reload preview + the
"Install as Flatpak with a user-approved capability manifest" step are
the GUI Forge, next (needs a display).
