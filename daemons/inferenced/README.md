# lisa-inferenced — model runtime & scheduler

Spec: docs/PLAN.md §5.1 — read it before changing this component (CLAUDE.md rule 1).

The one process that owns compute for inference: supervises engine children (llama-server, whisper.cpp, sd.cpp, ONNX), arbitrates VRAM/RAM with QoS classes, exposes D-Bus (org.lisa.Inference1) + an OpenAI-compatible endpoint on 127.0.0.1:7777. Runs with no network access — enforced by the systemd sandbox in os/packages, verified by the CI egress counter.

**M1 state:** real inference works — the llama engine supervises a llama-server child (spawn, /health-gated readiness, kill -9 recovery in ~2 s verified) and proxies streaming completions token-by-token; `lisa ask` produces real model output (`just smoke-real`). The stub engine remains for model-free tests. M1 remainder: QoS scheduler + preemption, guided generation (liblisa::grammar) threaded to sampling, multi-model residency, LoRA hot-swap, the D-Bus surface, and the perf budgets on reference hardware.
