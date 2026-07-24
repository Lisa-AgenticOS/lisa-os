<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="branding/lisa-wordmark-white.svg">
    <img alt="Lisa OS" src="branding/lisa-wordmark.svg" width="220">
  </picture>
</p>

<p align="center"><strong>An AI-native Linux distribution: local models as a system service, per-app context, MCP-native agents, an append-only Ledger.</strong></p>

<p align="center">
  <a href="https://github.com/Lisa-AgenticOS/lisa-os/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Lisa-AgenticOS/lisa-os/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/Lisa-AgenticOS/lisa-os/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/Lisa-AgenticOS/lisa-os?label=release&color=6D45C9"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-GPL--2.0-6D45C9.svg"></a>
</p>

---

> macOS gives you Apple's intelligence. Lisa gives you yours.

Lisa makes intelligence a **system service**. One daemon owns the GPU/RAM
budget and serves every app (`lisa-inferenced`); every app gets durable,
user-inspectable memory and scoped access to a personal context index
(`lisa-contextd`); every app exposes its actions over MCP
(`lisa-agentd`); remote/cloud models are reachable only through a single
ledgered egress broker (`lisa-remoted`); and an append-only **Ledger**
records every prompt, grant, and tool call — readable by you, enforced by
design, with network egress technically blocked for the daemons that hold
your data.

The name is a tip of the hat to the 1983 Apple Lisa — the GUI pioneer
that came before the Mac. This time the desktop gets the intelligence era
first.

## Status

**Alpha.** Lisa is a **bootable, self-updating, immutable OS** running a
GNOME desktop with AI shell surfaces — verified on a real **2017 iMac** as
well as QEMU. A/B slots with boot-counting rollback; GitHub Releases *are*
the update channel (`lisa update` pulls into the inactive slot). The full
plan and roadmap live in [`docs/PLAN.md`](docs/PLAN.md); decisions in
[`docs/adr/`](docs/adr/); current state in
[`docs/STATUS.md`](docs/STATUS.md).

What's live today:

- **Local inference** — `lisa-inferenced` supervises llama.cpp, streams
  real tokens, recovers from crashes, and does **guided generation**
  (JSON-Schema → GBNF, grammar-constrained output) with a QoS scheduler
  and multi-model residency. `lisa-modeld` is a blake3 content-addressed
  model store.
- **The assistant harness (ADR-0013)** — Siri-style **intents** (natural
  language → a grammar-guaranteed tool call, trust-checked at the bus) and
  a Claude-Code-style **coding agent** over a security jail, on Lisa's own
  Rust substrate.
- **Agent Bus** — `lisa-agentd`: apps are MCP servers; tools carry
  confirmation tiers, an undo journal, and provenance escalation, with a
  merge-gating injection suite (600 attempts, 0 unconfirmed privileged
  calls).
- **Bring-your-own cloud models** — 14 built-in providers (OpenAI,
  Anthropic, Moonshot, Gemini, DeepSeek, Groq, Mistral, xAI, OpenRouter,
  Perplexity, Together, Fireworks, HuggingFace, Tinker) behind the
  ledgered egress broker, with per-scope offload consent (default deny).
- **Trust & legibility** — the append-only Ledger (SQLite, UPDATE/DELETE
  blocked by triggers) enforced so no inference happens without an entry
  first; a portal trust boundary; a Ledger app in the shell.

```console
$ lisa ask "write a haiku about entropy"        # streams local tokens
$ git log | lisa ask "changelog, markdown"      # pipes are context
$ lisa do  "add milk to my notes"               # intent → consent → done
$ lisa ask --model remote:moonshot:kimi-k2 …    # BYO cloud, ledgered
$ curl 127.0.0.1:7777/v1/chat/completions ...   # any OpenAI client works
```

## Building

Requires Rust (stable) and [`just`](https://github.com/casey/just).

```console
$ just build   # cargo build --workspace
$ just test    # cargo test --workspace
$ just smoke   # end-to-end: daemon + lisa ask
$ just image   # mkosi OS image — Linux only, normally CI's job
```

## Layout

Monorepo per PLAN §9: `daemons/` (inferenced, modeld, contextd, agentd,
remoted), `portals/` (the trust boundary), `libs/` (liblisa SDK,
harness-core, forge-harness, lisa_ui, lisa-ledger), `shell/` (overlay,
launcher, Ledger app, settings), `apps/` (notes …), `cli/lisa`, `ime/`
(writing tools everywhere), `os/` (mkosi image + Track L layer),
`models/` (catalog), `tests/` (e2e, injection, perf, ACL fuzz).

## License

[GPL-2.0](LICENSE) — the same license as the Linux kernel this
distribution ships (ADR-0005). Model licenses are separate and reviewed
per-entry in the catalog before anything is offered for download.
