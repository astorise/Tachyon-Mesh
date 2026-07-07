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
`tachyon-v0.11.0-3`, [astorise/candle#8](https://github.com/astorise/candle/issues/8)),
but Tachyon does not yet own the required runtime state to use it: a CUDA KV
block pool, per-sequence block tables, and block-granular allocation/free
during continuous batching (tracked by `wire-paged-attention-decode-path`,
which also depends on this doc's "Single-Device GPU Execution" section above
for its CUDA baseline).

Model bindings can declare the future mode with:

```json
{
  "hardware_strategy": {
    "paged_attention": true
  }
}
```

Until the block allocator and block-table path are wired, the Candle LLM runtime
rejects that setting with a typed unsupported-model error instead of silently
falling back to the contiguous per-request KV cache.

## CUDA Graphs and FlashInfer Status

The pinned `astorise/candle` fork also includes the downstream APIs proposed in
`huggingface/candle#3651`: `candle_core::CudaGraph` for capture/replay and the
optional `candle-flashinfer-kernels` crate for FlashInfer-style decode
attention.

Model bindings can declare the future modes with:

```json
{
  "hardware_strategy": {
    "cuda_graph_decode": true,
    "flashinfer_attention": true
  }
}
```

Tachyon rejects both settings today. CUDA Graph replay requires a steady-state
GPU decode loop with fixed shapes and stable device buffers; FlashInfer requires
the decode-attention call site to pass single-token Q/K/V tensors to
`candle-flashinfer-kernels::flashinfer_decode_attention`. Until those runtime
paths are wired, the host fails closed instead of using the uncaptured/default
attention path.

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
