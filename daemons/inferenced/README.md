# lisa-inferenced — model runtime & scheduler

Spec: docs/PLAN.md §5.1 — read it before changing this component (CLAUDE.md rule 1).

The one process that owns compute for inference: supervises engine children (llama-server, whisper.cpp, sd.cpp, ONNX), arbitrates VRAM/RAM with QoS classes, exposes D-Bus (dev.lisaos.Inference1) + an OpenAI-compatible endpoint on 127.0.0.1:7777. Runs with no network access — enforced by the systemd sandbox in os/packages, verified by the CI egress counter.

**M1 state:** real inference works — the llama engine supervises a llama-server child (spawn, /health-gated readiness, kill -9 recovery in ~2 s verified) and proxies streaming completions token-by-token; `lisa ask` produces real model output (`just smoke-real`). The stub engine remains for model-free tests. Guided generation is live: OpenAI `response_format: json_schema` compiles to GBNF via liblisa and constrains the sampler (**1,000/1,000 valid** on the sampled acceptance gate, 2026-07-21). The QoS scheduler preempts background streams for interactive requests within the 250 ms budget (tested). M1 remainder: multi-model residency, LoRA hot-swap, the D-Bus surface, perf budgets on reference hardware.
