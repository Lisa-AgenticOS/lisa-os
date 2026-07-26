# colibrì evaluation — a second engine behind the `Engine` trait?

- **Date:** 2026-07-26 · evaluated against PLAN §5.1 (`lisa-inferenced`),
  §5.2 (`lisa-modeld`), §7 (model lineup), §8 (hardware tiers)
- **What it is:** [JustVugg/colibri](https://github.com/JustVugg/colibri) —
  **Apache-2.0** (not MIT), C engine + Python launcher/gateway. Runs
  **GLM-5.2 (744B MoE)** on ~25 GB of RAM by keeping the dense part
  resident and streaming the 19,456 routed experts from NVMe.

## Verdict

**Watch. Do not adopt, do not wire behind `Engine` yet.** Three
independent blockers, any one of which is sufficient today; §5 names the
signals that would flip this.

---

## 1. What it actually is

**Architecture.** GLM-5.2 activates ~40B of 744B parameters per token, and
only ~11 GB of those change token to token. So:

- **dense part** (attention, shared experts, embeddings, ~17B params) —
  resident in RAM at int4, **9.9 GB**;
- **19,456 routed experts** (75 MoE layers × 256 + MTP head, ~19 MB each) —
  on disk, **~370 GB**, streamed on demand;
- three tiers (VRAM / RAM / NVMe) with a per-layer LRU, a *learned* pinned
  hot-store (`.coli_usage`, rewritten every turn), and prefetch driven by
  running the router one layer ahead (claimed **71.6% predictable**).

Supporting tricks, all real in the source: adjacent storage of an expert's
three matrices so one `pread` fetches it; a bounded async I/O pool
(`PIPE=1`); batch-union (each unique expert read once per batch); dual-SSD
mirroring with deterministic hash-based, bandwidth-weighted routing; MLA
KV compression (576 floats/token vs 32,768) persisted across restarts;
GLM-5.2's native MTP head for speculative decoding at 2.2–2.8 tok/forward.

**Format.** *Not GGUF.* A bespoke int4 safetensors container (int8 MTP
heads), produced by their own Python converter from the FP8 upstream
release. Nothing in Lisa's GGUF world converts to it or from it.

**Runtime deps.** The "zero deps, no Python at runtime" line is **half
true and it is the wrong half for us**. The engine binary is C + OpenMP,
~368 KB, genuinely dependency-free. But the OpenAI/Anthropic HTTP
gateway — the only thing `lisa-inferenced` would ever talk to — is
[`c/openai_server.py`](https://github.com/JustVugg/colibri/blob/main/c/openai_server.py),
**1,695 lines of Python** (stdlib only) driving the engine over a
line-oriented stdin/stdout protocol
([serve_protocol.md](https://github.com/JustVugg/colibri/blob/main/docs/serve_protocol.md)).
The `coli` launcher is Python too. Adopting colibrì's server means
shipping Python in the inference path — squarely against §0.4 rule 4
("Python only for build tooling and evals"). The alternative is Lisa
speaking their `SUBMIT`/`DATA`/`DONE` mux protocol directly from Rust,
which is *feasible* (the protocol is documented and small) but is a
bespoke integration, not a drop-in.

The engine is also not the "1,300 lines" the press repeats:
`c/colibri.c` is **6,751 lines** plus ~3,800 lines of headers. Still
small and readable; just not the marketing number.

**Throughput — measured, from their own
[benchmarks.md](https://github.com/JustVugg/colibri/blob/main/docs/benchmarks.md):**

| machine | measured |
|---|---|
| 25 GB WSL2 dev box (the headline claim) | **0.05–0.1 tok/s** cold |
| Intel Core Ultra 7, 24 GB, WSL2 | 0.07 → 0.11 tok/s (`--topp 0.7`) |
| Apple M5 Max, 128 GB unified, Metal, 46.9 GB pin | **2.06 tok/s**, 72.5% hit |
| Mac Mini M4 Pro, 48 GB unified, Metal | 0.30 tok/s |
| Ryzen AI Max+ 395 (Strix Halo), 128 GB | 0.06 cold → 1.10–1.83 tok/s |
| Dell GB10 (aarch64 Grace, 121 GB unified) | 0.50 warm → 3.33 tok/s w/ `CACHE_ROUTE` |
| 6× RTX 5090, full residency (author's rig) | **5.8–6.8 tok/s**, TTFT ~13 s |

To their credit the project labels its estimates as estimates and its
measurements as measurements, and the headline number is the *floor*, not
a cherry-pick. But read it plainly: **the 25 GB configuration everyone is
excited about produces roughly one token every 10–20 seconds.** Their own
[api.md](https://github.com/JustVugg/colibri/blob/main/docs/api.md) says a
15k-token agent preamble is "an hour of silent thinking before the first
output token." That is not a system-AI daemon; that is a batch job.

**Quality.** Honestly reported and not flattering: int4 container scores
62.5% mean acc_norm on hellaswag/arc/mmlu (n=40), and their own
fp16-vs-int4 A/B on OLMoE puts the pure quantization cost at **−8.2 pp**
([#108](https://github.com/JustVugg/colibri/issues/108)).

## 2. Maturity and risk

| signal | value |
|---|---|
| repo created | **2026-07-01** — 25 days old |
| stars / forks | 19,064 / 1,886 (already past the 14.7k in the brief) |
| licence | **Apache-2.0** (the brief said MIT — wrong; weights are MIT) |
| contributors | 64 |
| commits | 623 on `main`; **100 in the last 14 days** |
| releases | 3 (v1.0.0 07-19, v1.1.0 07-22, v1.1.1 07-22) |
| issues | 212 total, 174 closed, 70 open; issue+PR numbering already past #622 |
| PRs | 350 closed |
| CI | 4 workflows; C test suite, CUDA + HIP syntax checks, release, site |
| tests | ~50 files in `c/tests/` — C unit tests + Python gateway tests, incl. `test_openai_tools_e2e.py`, `test_grammar.c`, `test_schema_gbnf.c` |

**The good.** For a 25-day-old project this is unusually disciplined.
Real CI. A dependency-free C test suite. Token-exact validation against a
`transformers` oracle (32/32 teacher-forcing). Issue templates with
labels, and a benchmark culture where community datapoints carry issue
numbers and get cited in docs. The v1.1.1 release note is a model of the
genre — it explains that 107 KB of zeros landed in `.data` because
`static GrDraft g_grd={.max=24};` moved a struct out of `.bss`, which is
what tripped Microsoft Defender. That is a maintainer who reads their own
object files.

**The bad.** Top contributor has 302 of ~620 attributed contributions and
the second-largest is 77; the last fortnight is 44 + 24 commits from the
same two identities out of 100. **Bus factor is effectively one, maybe
two.** Engine source comments are in **Italian** — fine for the author,
a real cost for anyone maintaining a fork on a 5-year OS support horizon.
Docs already drift from code:
[ENVIRONMENT.md](https://github.com/JustVugg/colibri/blob/main/docs/ENVIRONMENT.md)
says `GRAMMAR` and `SCHEMA` "constrain generation" — the code says the
exact opposite (§3 below); README's repo layout still names `c/glm.c`,
renamed to `colibri.c` in #391. And the whole thing shipped a binary
that antivirus flagged, three days after 1.0.0.

**Blunt read:** 19k stars in 25 days is a *distribution* signal, not a
*maturity* signal. There is no API stability commitment, no security
policy, no 1.x compatibility promise, and the on-disk container format is
one refactor away from changing.

## 3. Does it fit Lisa's `Engine` trait?

`Engine` needs four things (`daemons/inferenced/src/engine.rs`):
`generate` (streaming, optionally grammar-constrained), `embed`,
`raw_chat` (verbatim OpenAI body for tool calls), `shutdown`.

| requirement | colibrì | verdict |
|---|---|---|
| OpenAI-compatible server | yes — `coli serve`, `/v1/chat/completions`, `/v1/completions`, `/v1/models`, SSE streaming, usage counts, `/health` | ✅ **but** the server is Python |
| readiness probe | `GET /health` (active/queued/completed/rejected) | ✅ maps to `wait_healthy` |
| streaming | SSE, same delta shape `llama.rs` already parses | ✅ |
| **native tool calling** | yes on both `/v1/chat/completions` and `/v1/messages`: `tools`/`functions`, all `tool_choice` modes, `tool_calls` in responses | ⚠️ see below |
| **grammar / guided generation** | `response_format` accepts `json_object`, `json_schema`, and raw `gbnf` | ❌ **see below — this is the blocker** |
| embeddings | **none.** Zero occurrences of "embed" in the gateway; no `/v1/embeddings` endpoint exists | ❌ |
| single-model, single-stream | one model per process; concurrent requests hit a bounded FIFO admission queue (`--max-queue`, default 8) and get 429s | ⚠️ no continuous batching across models |
| shutdown | child process, EOF on stdin = graceful drain | ✅ |

### The grammar blocker

This is disqualifying for the local lane as it stands.
`c/grammar.h`'s own header comment (translated from the Italian):

> *"The grammar NEVER constrains sampling: it only proposes drafts, which
> verification accepts or rejects like any other draft. A wrong or
> out-of-sync grammar ⇒ rejected drafts, IDENTICAL output. It is a pure
> accelerator, never a filter."*

[grammar-draft.md](https://github.com/JustVugg/colibri/blob/main/docs/grammar-draft.md)
confirms it at the API level: `response_format` is *"a draft source, never
a sampling constraint."* It is a clever idea — in a disk-streaming MoE,
grammar-forced spans convert directly into expert reads avoided, so it
pays more here than in a dense engine — but it is the wrong primitive for
us. Lisa's §5.1 acceptance block reads *"given a JSON Schema, 1,000
sampled outputs → 100% parse + validate."* Colibrì's design **cannot
meet that**: passing a schema changes speed, not validity. `lisa do` on
local models, and liblisa's JSON-Schema→GBNF module (ROADMAP, §5.1
acceptance), would have no enforcement path.

Worse, it fails *quietly*. `llama.rs` sets `body["grammar"]` and gets
constrained sampling; the equivalent colibrì request returns unconstrained
text with an HTTP 200. Wiring it behind `Engine` without a capability flag
would silently degrade every guided call.

### The tool-calling caveat

Tool calling works, but it is implemented in the **Python gateway**, not
the engine: `render_chat` writes tool declarations into the prompt and
`parse_tool_calls` regex-parses GLM's markers back into OpenAI
`tool_calls`, with a `COLI_TOOL_SALVAGE=1` de-mangler for when the model
half-closes a tag. That is functionally what `--jinja` does for us — but
hardcoded to GLM's marker syntax rather than driven by the model's own
chat template. Any other model family needs new parser code. (Note
`api.md` still claims tools "return an explicit error"; the code
contradicts it — more doc drift.)

### The embeddings gap

Not a caveat, an absence. Lisa needs `embed` for `lisa-contextd`.
Colibrì would need a sibling `llama-server` for embeddings anyway, which
means colibrì can only ever be an *additional* engine, never a
replacement.

## 4. Hardware fit for Lisa's reference targets

**Field iMac18,2 (16 GB RAM, Radeon Pro 560): no.** Three ways:

1. **The model doesn't fit the machine's role.** GLM-5.2 int4 is **372 GB
   on disk**. The iMac was just re-imaged onto "the bigger disk"
   (STATUS.md) and is a general-purpose desktop — dedicating 372 GB to one
   model's weights is not a thing a device OS does to a user's disk.
2. **The GPU is dead weight.** Colibrì has CUDA and Metal backends and a
   HIP/ROCm syntax check in CI. Radeon Pro 560 is Polaris (gfx804), which
   modern ROCm does not support. No Vulkan backend exists. So the iMac
   runs the CPU path only — and it has *less* RAM than the 24–25 GB boxes
   that measure 0.07 tok/s.
3. **Even if it fit, it's the wrong shape.** §5.1's budget is "`lisa ask`
   streams tokens from a cold boot in < 3 s." Colibrì's own docs describe
   hour-long prefills.

**aarch64 lane (Apple Silicon under virtualisation): the good number is
not available to us.** The 2.06 tok/s M5 Max figure is the **Metal**
backend on bare macOS with 128 GB unified memory and a 46.9 GB learned
pin. Lisa's aarch64 lane is a *Linux guest under QEMU+HVF* — no Metal, no
GPU passthrough, and disk I/O goes through virtio, exactly the path that
turned a 430 GB EPYC box into ~1 GB/s in
[#104](https://github.com/JustVugg/colibri/issues/104). Their own dev
baseline is a WSL2 VHDX for the same reason: virtualised storage is where
this architecture hurts most. The genuinely encouraging aarch64 datapoint
is the Dell GB10 / Grace box (3.33 tok/s,
[#136](https://github.com/JustVugg/colibri/issues/136)) — but that is
121 GB of unified LPDDR5x and 5.58 GB/s O_DIRECT on bare metal.

**Where it *would* shine: §8 Tier 4.** Strix Halo-class unified-memory
machines — Lisa's declared flagship co-marketing target (§13, item 7) —
are exactly colibrì's best CPU-only class (1.10–1.83 tok/s measured on a
Ryzen AI Max+ 395, [#200](https://github.com/JustVugg/colibri/issues/200)).
If Lisa ever wants a "run a frontier model on your desk" halo feature,
Tier 4 + colibrì is the only credible path today. It is a demo, not a
default.

**SSD wear.** Their
[benchmarks.md](https://github.com/JustVugg/colibri/blob/main/docs/benchmarks.md#ssd-note)
addresses this directly and, I think, correctly: expert streaming is
**read-only**, and reads do not consume NAND write endurance. The real
risks they name are (a) swap traffic if the RAM budget is set too high —
writes, and those do wear — and (b) sustained thermals from hours at full
read duty cycle. For a device OS I'd add a third they don't: **~11 GB of
random reads per token means the page cache is thrashed continuously**,
which evicts everything else the desktop cares about. On a 16 GB machine
that is a system-wide responsiveness cliff, not just a slow model. Any
future integration must run in its own cgroup with a memory ceiling —
which §5.1 already mandates, so the mechanism exists.

## 5. Recommendation

**Watch.** Not adopt, not "add as a second engine" — *yet*. It is the
most interesting systems idea in local inference this year and the
engineering culture around it looks real, but on 2026-07-26 it cannot
satisfy §5.1's acceptance block, cannot embed, ships Python in the serving
path, has a bus factor of one, and its flagship configuration is a token
every fifteen seconds on hardware we don't target.

`Engine` is exactly the right seam for it, and nothing here argues against
that design — the trait is why this is a cheap decision to defer.

**Signals that would change the answer** (any two of the first three
would make me revisit; #1 alone unblocks the local lane):

1. **Constrained sampling, not just drafting.** A `response_format` /
   `grammar` mode that actually filters the sampler, with a 100%
   parse+validate claim. Track
   [grammar-draft.md](https://github.com/JustVugg/colibri/blob/main/docs/grammar-draft.md)
   and `c/grammar.h`; today's code says "never a filter" in as many words.
2. **A native (non-Python) server**, or a stable, versioned commitment to
   the `SUBMIT`/`DATA`/`DONE` mux protocol we could implement from Rust.
   The protocol doc already states a forward-compatibility rule ("ignore
   line kinds you don't recognize") — a semver promise on top of it would
   be enough.
3. **A model in Lisa's actual size class.** Their roadmap names
   **Qwen3 MoE** — `Qwen3-30B-A3B` is already Lisa's §7 Tier 3/4 "big"
   system model. A 30B-A3B running under colibrì's tiering on a 16 GB
   Tier 1/2 box, at interactive speed, is the datapoint that would make
   this a product decision instead of a curiosity.
4. **Second maintainer with commit rights and sustained volume**, or a
   foundation/vendor backstop. Re-check contributor distribution at the
   90-day and 180-day marks.
5. **A Vulkan backend.** Would light up the iMac's Radeon and every other
   non-CUDA, non-Metal GPU we support. Nothing in the repo suggests this
   is planned.
6. **Format stability.** The int4 safetensors container needs a version
   field and a compatibility promise before `lisa-modeld` could pin blake3
   hashes against it (§5.2) — and the only published GLM-5.2 conversion
   lives on a **third-party** HF account
   ([mateogrgic/GLM-5.2-colibri-int4-with-int8-mtp](https://huggingface.co/mateogrgic/GLM-5.2-colibri-int4-with-int8-mtp),
   created 2026-07-10, 10.9k downloads), which is not a provenance story
   §5.2 can accept.

**Cheap thing to do now:** nothing in the repo. Re-read this note when
signal #1 or #3 lands.

---

## Claims I could not verify

- **"14.7k stars, MIT, released 2026-07-10."** All three are off. The
  GitHub API reports **19,064 stars**, **Apache-2.0**, repo created
  **2026-07-01**. (MIT is the *GLM-5.2 weights* licence, per their README;
  07-10 is when the HF conversion was published and press coverage began.)
- **"~1,300 lines of pure C."** `c/colibri.c` is 6,751 lines; headers add
  ~3,800. Widely repeated in coverage; not what the repo contains.
- **Every performance number in §1** is *their* measurement or a community
  datapoint filed as a GitHub issue. Nothing here was reproduced by us —
  we have no machine with 372 GB free and no GLM-5.2 conversion on disk.
- **"71.6% router predictability one layer ahead"** and the token-exact
  `transformers` oracle validation are asserted in the README; the test
  harness exists (`c/tools/make_glm_oracle.py`, `ref_glm.json`) but we did
  not run it.
- **Contributor counts** are GitHub's attribution, which merges some
  identities oddly (a `claude` account with 4 commits, a "Hermes Quant
  Auditor" with 2) — treat the 64 figure as an upper bound on humans.

## Sources

All fetched and verified to resolve on 2026-07-26.

- https://github.com/JustVugg/colibri
- https://github.com/JustVugg/colibri/blob/main/docs/api.md
- https://github.com/JustVugg/colibri/blob/main/docs/grammar-draft.md
- https://github.com/JustVugg/colibri/blob/main/docs/benchmarks.md
- https://github.com/JustVugg/colibri/blob/main/docs/serve_protocol.md
- https://github.com/JustVugg/colibri/blob/main/docs/metal.md
- https://github.com/JustVugg/colibri/blob/main/c/grammar.h
- https://github.com/JustVugg/colibri/blob/main/c/openai_server.py
- https://github.com/JustVugg/colibri/releases/tag/v1.1.1
- https://github.com/JustVugg/colibri/issues/108 (quality ablation)
- https://github.com/JustVugg/colibri/issues/136 (aarch64 / GB10)
- https://github.com/JustVugg/colibri/issues/200 (Strix Halo)
- https://justvugg.github.io/colibri
- https://huggingface.co/mateogrgic/GLM-5.2-colibri-int4-with-int8-mtp
