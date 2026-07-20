# Lisa OS

**An AI-native Linux distribution: local models, system-wide context,
agentic apps.**

> macOS 27 gives you Apple's intelligence. Lisa gives you yours.

Lisa makes intelligence a system service. One daemon owns the GPU/RAM
budget and serves every app (`lisa-inferenced`); every app gets durable,
user-inspectable memory and scoped access to a personal context index
(`lisa-contextd`); every app exposes its actions over MCP
(`lisa-agentd`); and an append-only **Ledger** records every prompt,
grant, and tool call — readable by the user, enforced by design, with
network egress technically blocked for the daemons that hold your data.

The name is a tip of the hat to the 1983 Apple Lisa — the GUI pioneer
that came before the Mac. This time the desktop gets the intelligence
era first.

## Status

**Pre-alpha, milestone M0 (bootstrap).** The full plan — vision,
architecture, component specs, roadmap — lives in
[`docs/PLAN.md`](docs/PLAN.md). Decisions are logged in
[`docs/adr/`](docs/adr/).

What works today:

- Cargo workspace: `lisa-inferenced` (OpenAI-compatible endpoint on
  `127.0.0.1:7777`, stub engine, llama-server supervision scaffold),
  `lisa-modeld` (blake3 content-addressed model store with dedupe,
  verify, gc), `liblisa` (JSON-Schema → GBNF guided-generation module),
  and the `lisa` CLI.

```console
$ lisa ask "write a haiku about entropy"        # streams tokens
$ git log | lisa ask "changelog, markdown"      # pipes are context
$ curl 127.0.0.1:7777/v1/chat/completions ...   # any OpenAI client works
```

Real model inference lands in M1 (see the roadmap in PLAN §10).

## Building

Requires Rust (stable) and [`just`](https://github.com/casey/just).

```console
$ just build   # cargo build --workspace
$ just test    # cargo test --workspace
$ just smoke   # end-to-end: daemon + lisa ask
$ just image   # mkosi OS image — Linux only, normally CI's job
```

## Layout

Monorepo per PLAN §9: `daemons/` (inferenced, modeld, contextd, agentd),
`portals/` (the trust boundary), `libs/` (liblisa SDK, lisa_ui,
forge-harness), `shell/` (overlay, launcher, Ledger app), `apps/`,
`cli/lisa`, `ime/` (writing tools everywhere), `os/` (mkosi image +
Track L layer), `models/` (catalog), `tests/` (e2e, injection,
perf, ACL fuzz).

## License

[GPL-2.0](LICENSE) — the same license as the Linux kernel this
distribution ships (ADR-0005). Model licenses are separate and reviewed
per-entry in the catalog before anything is offered for download.
