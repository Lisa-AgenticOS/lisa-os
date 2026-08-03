# lisa-inferenced — model runtime & scheduler

Spec: docs/PLAN.md §5.1 — read it before changing this component (CLAUDE.md rule 1).

The one process that owns compute for inference: supervises engine children (llama-server, whisper.cpp, sd.cpp, ONNX), arbitrates VRAM/RAM with QoS classes, exposes D-Bus (dev.lisaos.Inference1) + an OpenAI-compatible endpoint on 127.0.0.1:7777 — and, with `--socket`, the same API on a unix socket for callers that are forbidden IP sockets outright (`lisa-contextd` embeds this way: `RestrictAddressFamilies=AF_UNIX` means loopback is out of reach too, #163). Runs with no network access — enforced by the systemd sandbox in os/packages, verified by the CI egress counter.

**M1 state:** real inference works — the llama engine supervises a llama-server child (spawn, /health-gated readiness, kill -9 recovery in ~2 s verified) and proxies streaming completions token-by-token; `lisa ask` produces real model output (`just smoke-real`). The stub engine remains for model-free tests. Guided generation is live: OpenAI `response_format: json_schema` compiles to GBNF via liblisa and constrains the sampler (**1,000/1,000 valid** on the sampled acceptance gate, 2026-07-21). The QoS scheduler preempts background streams for interactive requests within the 250 ms budget (tested). M1 remainder: multi-model residency, LoRA hot-swap, the D-Bus surface, perf budgets on reference hardware.

**Remote routing (§5.11, ADR-0010):** `remote:<provider>:<model>` names route to the `lisa-remoted` egress broker over its unix socket — inferenced keeps zero network access (rule 5). Streaming is real end to end: the request carries `stream:true`, and the broker's SSE response (already normalized to OpenAI `chat.completion.chunk` frames for every provider dialect) is decoded incrementally — hand-rolled HTTP/1.1 head parse, chunked-transfer decode, SSE frames — and yielded as true token deltas through the engine stream. Mid-stream `{"error":...}` frames and early EOF surface as engine errors; a 150 s idle read timeout prevents hangs. Consent and the `remote.` Ledger marking are enforced broker-side.

**Multimodal input (#209).** A message's `content` is either a plain
string or an array of OpenAI **content parts** — `image_url`,
`input_audio`, whatever a provider adds next. Parts are carried as
opaque JSON and passed through verbatim rather than re-modelled: a
schema we own would need a release per modality and would silently
drop anything unmodelled, which is the worst failure here because a
dropped image still gets a confident answer about a picture nobody
saw. The local llama engine **refuses** a request carrying non-text
parts, naming what it cannot see, instead of flattening the image
away and answering anyway; remote engines forward the parts as they
arrived. Text-only callers are untouched: a bare string still
serializes as a bare string on the wire.
