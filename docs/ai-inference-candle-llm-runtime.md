# Candle LLM Runtime

Tachyon's `core-host --features ai-inference` build supports a real Candle-backed
LLM path for local text-generation model bindings. This path is separate from
legacy Candle ONNX/WASI-NN graph loading and from the ModelOpt/NVFP4 loader.

## Supported Fixture Family

The first supported family is `TachyonTinyCausalLM`, identified by:

```json
{
  "model_type": "tachyon_tiny_causal_lm",
  "architectures": ["TachyonTinyCausalLM"]
}
```

The model directory must contain:

- `config.json`
- `tokenizer.json`
- `model.safetensors`

`model.safetensors` must contain an F32 tensor named `next_token_logits` with
shape `[vocab_size]`. The runtime loads the tokenizer with `tokenizers`, loads
the tensor through Candle safetensors support, and performs deterministic greedy
selection with Candle `argmax`.

This fixture is intentionally small so CI can validate real Candle execution
without downloading external artifacts.

## Prompt Input

The first `U8` inference tensor is interpreted as either plain UTF-8 prompt text
or a JSON generation request.

Plain prompt:

```text
hello
```

JSON request:

```json
{
  "prompt": "hello",
  "max_new_tokens": 1,
  "temperature": 0.0,
  "seed": 7
}
```

Default generation is deterministic. Requests are bounded by prompt byte count,
prompt token count, batch size, and the host max-new-token cap.

## Incremental Detokenization

Every decode step needs the text generated so far — to test stop sequences, and
to emit the streaming delta. Decoding the whole token sequence on each step is
the obvious way to get it and is what the loops used to do, but it is O(n²) in
the generated length: at 256 tokens that is invisible, at several thousand it is
millions of redundant per-token decodes.

A naive fix does not work. A BPE/SentencePiece decode is not the concatenation
of its tokens' decodes: a multi-byte character can span several tokens (the
prefix decodes to a replacement character that a later token retroactively
replaces), and decoding a sub-sequence can gain or lose a leading space.

`IncrementalDecoder` keeps a bounded trailing *window* of tokens
(`DETOKENIZE_WINDOW_TOKENS`), re-decodes only that window, and appends the
difference against the previous window decode — so any sub-sequence artifact is
present in both decodes and cancels. When the new token revises text inside the
window, the decoder replaces that suffix wholesale instead of appending. The
window is re-anchored only when the shorter decode is *verifiably* a suffix of
the current one, so a tokenizer that never offers a clean split keeps a growing
window and degrades to the old whole-sequence behaviour rather than corrupting
output; a failed re-anchor backs off so it is not retried every step.

The invariant — that the incremental text equals what decoding the whole
sequence would produce — is asserted step by step in
`incremental_detokenization_matches_the_model_tokenizer` against the real
fixture tokenizer, and against a synthetic byte-level tokenizer for the
split-character case.

Stop-sequence scanning is deliberately left as a full-text search per step: it
is also O(n²), but it is a `memchr`-backed substring search over bytes, orders of
magnitude cheaper per byte than tokenizer work, and keeping it whole-text avoids
any question about missing an earlier match.

## Mock and Unsupported Bindings

Mock output is only available through explicit mock bindings such as:

```text
mock:llama3
```

Non-mock model bindings that are not supported Candle LLM directories, ONNX
guest-loaded graphs, or ModelOpt/NVFP4 directories fail during initialization
with an actionable error. They do not fall back to `MOCK_LLM_RESPONSE`.

## NVFP4 Boundary

ModelOpt/NVFP4 remains a separate loader and kernel capability. A detected
directory is never interpreted as Llama merely because it contains
safetensors. The registered Qwen 3.5 MoE compatibility profile executes through
its dedicated hybrid runtime; unmatched architectures return an explicit
unsupported-architecture error.

## Single-Device GPU Execution

The single-device (`GpuDistribution::Single`) path executes on a real CUDA
device only for a Llama-family checkpoint, and only on a build with the
`candle-cuda` feature: `load_safetensors` resolves
`Device::cuda_if_available(0)` when the binding requests a non-`cpu` device
for a Llama checkpoint, falling back to `Device::Cpu` silently if no
physical GPU is found at runtime (the same convention the tensor/pipeline/
expert-parallel engines already use). Every other architecture on the
single-device path — Qwen2/3, Gemma2/3, Phi3/4, the DeepSeek family — and
every build without `candle-cuda` still return the existing typed
`UnsupportedModel` error for a non-`cpu` request; multi-GPU placement
already had its own real CUDA path via `distribution_mode:
tensor_parallelism`/`pipeline_parallelism`/`expert_parallelism`, unaffected
by this.

### Quantized GGUF on CUDA

GGUF follows the same rule. `load_gguf` resolves the binding's device rather
than pinning `Device::Cpu`, so a Llama GGUF bound to a non-`cpu` device on a
`candle-cuda` build uploads its quantized weights to the GPU and decodes there;
`LoadedModel::Gguf` carries that device so prefill and decode build their input
tensors on it. This is what makes a 4-bit checkpoint usable on a consumer GPU:
the safetensors path loads F32 (or BF16 under `paged_attention`), which costs
4 (or 2) bytes per parameter, while a Q4_K_M GGUF costs roughly half a byte.

Only block types with a CUDA quantized-matmul kernel in the pinned fork are
accepted: `Q4_0`, `Q4_1`, `Q5_0`, `Q5_1`, `Q8_0`, `Q2K`, `Q3K`, `Q4K`, `Q5K`,
`Q6K`, plus `F32`/`F16`/`BF16` (which `QMatMul::from_arc` dequantizes into a
dense tensor rather than dispatching to a quantized kernel). `Q8_1` and `Q8K`
dequantize on CUDA but have no matmul kernel, so a checkpoint carrying them is
rejected at load — with the offending tensor and block type named — instead of
failing at the first decode step with VRAM already claimed. A `cpu` binding
skips the check entirely and keeps the historical host path.

`paged_attention`, `cuda_graph_decode`, and `flashinfer_attention` remain
safetensors-only and are rejected for a GGUF binding: `QuantizedLlama` owns its
KV cache and decode path internally, never sees `hardware_strategy`, and exposes
no block-table seam. Accepting any of them for GGUF would silently run ordinary
quantized attention while reporting the optimization as enabled.

One subtlety: the block-type check runs *after* the device is resolved, not
before. `Device::cuda_if_available` falls back to `Device::Cpu` when no physical
GPU is present, and a checkpoint that lands on that fallback executes through the
host kernels, which support every block type — scanning first would reject a
perfectly runnable model on a `candle-cuda` build with no GPU attached.

## OpenAI-Compatible Upstream Bindings

A model binding whose `path` uses the `openai:` scheme is served by an external
OpenAI-compatible server rather than by a local Candle runtime. The mesh keeps
routing, QoS, authorisation, and the `/ai/v1` surface; only the tensor math runs
out of process. This is the supported way to serve a model whose architecture or
quantization Tachyon has no verified loader for — an AWQ checkpoint under vLLM,
or a non-Llama GGUF under `llama-server`.

```json
{
  "alias": "qwen3-coder",
  "path": "openai:http://127.0.0.1:8080/v1?model=qwen3-coder-30b&timeout_ms=180000"
}
```

- The upstream model name defaults to the binding alias; `model` overrides it,
  so a mesh alias need not match the upstream's own name.
- `timeout_ms` bounds each request (default 5 minutes, ceiling 1 hour).
- `max_new_tokens` sets this binding's generation budget (default 2048, ceiling
  `UPSTREAM_MAX_NEW_TOKENS` = 8192).
- Credentials never appear in the binding. The runtime reads a bearer token from
  `TACHYON_UPSTREAM_API_KEY_<ALIAS>` (alias upper-cased, non-alphanumerics
  folded to `_`), falling back to `TACHYON_UPSTREAM_API_KEY`. With neither set,
  the request goes out unauthenticated — the usual case for a `llama-server` on
  a trusted mesh link.

The host's JSON generation request maps onto the chat-completions body:
`messages` is forwarded verbatim (the upstream applies its own chat template,
since the model lives there), `prompt` becomes a single user turn,
`max_new_tokens` becomes `max_tokens`, and `json_schema` becomes an OpenAI
`response_format`. Streaming is real SSE passthrough — one token per content
delta — so time-to-first-token survives the extra hop. LoRA adapters are
rejected: adapter injection has no wire representation here.

A generation budget is always sent, so an absent `max_new_tokens` cannot leave
the upstream's own (possibly unlimited) default in charge. The bound is the
binding's own, **not** the native runtime's `HOST_MAX_NEW_TOKENS`, because the
two limits protect different resources: the native cap bounds this host's decode
loop, where every extra token is a forward pass and more KV cache in local VRAM,
while an upstream generation costs this node one open HTTP connection and spends
the *remote* server's resources, which enforces its own context window and
queueing. Applying the 256-token native cap here would truncate an agentic
completion mid-function for no local benefit.

So upstream bindings default to 2048 tokens, are capped at
`UPSTREAM_MAX_NEW_TOKENS` (8192), and can be tuned per alias with
`?max_new_tokens=`. The per-binding override is itself bounded, so a typo cannot
make a request effectively unlimited.

A batch is dispatched concurrently, one scoped thread per request, and results
are returned per input. Both matter here and not for local runtimes: these are
independent network round trips on the accelerator dispatcher thread shared by
every CPU-resident model, so running them sequentially would make the last
caller wait for the sum of all preceding upstream latencies and block unrelated
local inference meanwhile; and an upstream failure is usually about one request
(a rejected prompt, a 400), so collapsing the batch into a single `Result` would
fail every co-batched caller and discard responses already received.

Every read is bounded — response bodies, error excerpts, whole SSE streams, and
individual SSE frames — and a stream that ends before its `[DONE]` sentinel is
reported as a truncated generation rather than a successful one, so an upstream
that restarts mid-response cannot hand the caller silently truncated code.
Embedding components that do not narrow to a finite `f32` are rejected instead
of becoming infinities that turn downstream cosine similarities into NaN.

Binding validation is offline, so a node still boots when its upstream is down;
an unreachable server is a per-request error naming the endpoint and status, and
an error body is never returned as generated text. Because no weights are
resident locally, an upstream binding reports no local accelerator residency
whatever `device` it declares.

## Cross-Node Model Placement Watchlist

Tachyon currently optimizes for homelab/edge deployments where the target model
fits within one RTX-class node. Multi-GPU model parallelism is active only as a
single-node placement concern. `parallel.rs` contains the NCCL all-reduce path
for CUDA tensor-parallel ranks and a minimal TCP stage transport that can move
pipeline activations over a real socket, but Tachyon does not treat placement of
one live model across multiple machines as an active product requirement.

Keep production cross-machine placement of one live model in watchlist status
until a roadmap model exceeds the aggregate VRAM capacity of a single target
node, for example a 100B+ unquantized checkpoint or an unusually long-context
deployment. The existing NCCL TCP bootstrap and `StageTransport` socket
primitives remain valid groundwork, but they are not by themselves a product
requirement to orchestrate one forward pass across machines. Until the roadmap
trigger is met, scale horizontally by overflowing whole requests to peers that
already host the model. On reactivation, reassess how far
`discover_cluster_topology()`, `parallel.rs`, and the TCP bootstrap cover the
target topology before sizing placement, NUMA binding, peer failure during
forward, and production orchestration work.

## PagedAttention Status

The pinned `astorise/candle` fork is consumed through a Tachyon release tag
(bumped as the fork gains capabilities Tachyon depends on — see
`core-host/Cargo.toml` for the current pin), not a raw commit rev, so Renovate
can track the git ref. Each fork refresh should rebase the fork on the selected
upstream Candle commit, run the Candle/Tachyon AI inference checks, publish a new
`tachyon-v<upstream-version>-<N>` tag, and update `core-host/Cargo.toml` plus
`Cargo.lock` together. The fork includes Candle's paged flash-attn API
(`flash_attn_varlen_paged_windowed`) and an additive `Cache::set_paged_kv`/
`paged_kv` per-layer seam in `candle-transformers::models::llama` (tag
`tachyon-v0.11.0-3`, [astorise/candle#8](https://github.com/astorise/candle/issues/8)).

`hardware_strategy.paged_attention: true` is enabled for a **Llama checkpoint
on a CUDA device** (see "Single-Device GPU Execution" above for that
baseline): `core-host/src/ai_inference/paged_kv.rs` owns a `PagedBlockPool`
(fixed-size free-list allocator) and a per-request `SequenceBlockTable`;
`candle_llm_runtime.rs` allocates one `(key_cache, value_cache)` tensor pair
per transformer layer, sized from real NVML free-VRAM telemetry (a fixed
heuristic — 50% of free VRAM remaining after weight load, with a floor of
one full-length sequence at the checkpoint's `max_position_embeddings` —
not yet a configurable `hardware_strategy` knob), and attaches
`cache.set_paged_kv(..)` for every layer on every forward step. Every other
architecture, and any non-CUDA device or build without `candle-cuda`, still
returns the existing typed unsupported-model error instead of silently
falling back to the contiguous per-request KV cache.

**BF16, not F32**: `candle-flash-attn`'s kernels only support F16/BF16 (an
industry-standard constraint of fused attention kernels, not specific to
Tachyon), so a paged-attention Llama deployment loads its weights and KV
cache in BF16 instead of the contiguous path's F32. Generation is a real,
deterministic decode (repeating the same greedy request yields identical
output), but is not expected to be bit-identical to the F32 contiguous path.

Not yet supported in combination with paged attention: speculative decoding
(falls back to plain generation rather than erroring, since verification
would otherwise dtype-mismatch against the BF16 model) and continuous
batching across concurrent sequences (each request gets its own block table
today; the shared pool is reused across requests, but only one sequence is
in flight per forward pass). LoRA adapters are unaffected either way — they
already reload an independent CPU/F32 model per request.

Model bindings can opt in with:

```json
{
  "hardware_strategy": {
    "paged_attention": true
  }
}
```

## FlashInfer Decode Attention Status

The pinned `astorise/candle` fork carries an additive `use_flashinfer_attention`
seam on `candle_transformers::models::llama::Config` (mirroring the existing
`use_flash_attn` flag), wired to the optional `candle-flashinfer-kernels`
crate's `flashinfer_decode_attention`.

A Llama model binding on a CUDA device with the `candle-flashinfer` Cargo
feature compiled in can opt in with:

```json
{
  "hardware_strategy": {
    "flashinfer_attention": true
  }
}
```

The decode step (one query token per sequence) then attends via
`candle_flashinfer_kernels::flashinfer_decode_attention` against the
pre-`repeat_kv` contiguous KV cache instead of the dense matmul+softmax (or
`flash_attn`) path; prefill is unaffected. Unlike `paged_attention`, this needs
no dtype switch — a flashinfer-attention deployment stays on the same F32 path
as the plain dense Llama path, since the kernel supports F32/F16/BF16 with no
block-size or head-dim alignment requirement. `flashinfer_attention` cannot be
combined with `paged_attention` in the same deployment (they select different
decode-attention kernels over different KV cache layouts) — that combination
is rejected with a typed error rather than silently picking one. Every other
combination (non-Llama architecture, non-CUDA device, or a build without
`candle-flashinfer` compiled in) keeps the existing typed rejection. See
`openspec/changes/wire-flashinfer-decode-attention`.

## CUDA Graph Decode Status

The fork carries `candle_core::CudaGraph` (capture/replay) plus two additive
seams a captured decode step needs: `Cache::set_decode_position` (rotary
embeddings read a persistent, in-place-updatable device tensor instead of the
host-side `narrow(0, index_pos, seq_len)` a replayed graph would otherwise
silently bake in) and `Cache::set_paged_kv_decode_slot` (the KV-cache scatter
destination is likewise a persistent device tensor the caller updates in
place, instead of `PagedKvCache::write_new_kv` deriving it via a blocking
device-to-host block-table readback — itself not capturable at all).

A Llama model binding on a CUDA device can opt in with both flags together:

```json
{
  "hardware_strategy": {
    "paged_attention": true,
    "cuda_graph_decode": true
  }
}
```

`cuda_graph_decode` requires `paged_attention: true` (typed error naming the
dependency when missing) — the contiguous KV cache's per-step reallocation
via `Tensor::cat` is incompatible with CUDA graph replay, and the
pre-allocated `PagedKvCache` layout is the only candidate this runtime has.
Once both flags are set on a supported build, `CudaGraphDecodeSession` runs
the steady-state decode step (post-prefill, one token at a time) through a
real captured/replayed graph: the block-table/seqlens/position/decode-slot
tensors are sized once at their full maximum width and updated only via
`Tensor::slice_set`, `model.forward`'s `index_pos` argument becomes
irrelevant once both position/decode-slot seams are attached (the real
position flows through those device tensors instead), and every decode step
after the first captured one is a single `CudaGraph::replay()` call. Prefill,
LoRA adapters, and speculative decoding are unaffected (prefill always uses
the existing uncaptured path — chunk shapes vary chunk to chunk; speculative
decoding already falls back to plain generation for any paged-attention
target/draft). Every other combination (non-Llama architecture, non-CUDA
device, or `cuda_graph_decode` without `paged_attention`) keeps the existing
typed rejection.

A capture, warm-up, and however many replays a request needs are correct on
real GPU hardware, with output identical to the non-captured paged-attention
path for the same prompt. A second independent request against the same
loaded runtime is also proven: PR #367's `cuda-quality` job runs the two
requests in sequence and asserts identical greedy output. This relies on
`astorise/candle` `tachyon-v0.11.0-10` and its matching `cudarc` patch, which
keep event tracking paused for the graph lifetime and safely tear down
graph-owned CUDA allocations before another request creates a cache.

## Speculative Decoding Status

Model bindings can opt into greedy draft/verify decoding with a local draft
model directory:

```json
{
  "hardware_strategy": {
    "speculative_draft_model_path": "/models/tiny-draft",
    "speculative_draft_tokens": 4
  }
}
```

The draft model proposes a bounded token window and the target model verifies
each token before emission. This first path is exact for greedy single-prompt
generation; sampling, constrained decoding, tokenizer-incompatible drafts, and
multi-prompt batches fall back to the existing target-only decode path.
