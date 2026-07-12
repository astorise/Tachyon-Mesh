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

**Known limitation**: only the *first* request served against a freshly
loaded `cuda_graph_decode` model is currently proven to work. A capture,
warm-up, and however many replays a single request needs are all correct —
confirmed on real GPU hardware, output identical to the non-captured
paged-attention path for the same prompt. A **second, independent request
against the same already-loaded model currently fails** with a low-level
CUDA driver error (`CUDA_ERROR_INVALID_VALUE`) on an unrelated allocation,
suspected to be `candle-core`-internal event-tracking state left
inconsistent by the first request's `CudaGraph::capture`/teardown cycle —
not something inspectable or fixable from Tachyon-Mesh's side. Tracked as
[astorise/candle#17](https://github.com/astorise/candle/issues/17). Until
that's resolved (or a caller-side mitigation is added), **do not enable
`cuda_graph_decode` for any model expected to serve more than one request**
— which is effectively every production deployment today. See
`openspec/changes/wire-cuda-graph-decode` for the full investigation.

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
