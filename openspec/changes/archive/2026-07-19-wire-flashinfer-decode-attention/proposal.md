## Why

Issue #312 step 3: `hardware_strategy.flashinfer_attention` is fully plumbed
through manifest, schema, UI, and MCP, but `CandleLlmRuntime::try_load_with_topology`
rejects every request for it unconditionally, naming
`candle-flashinfer-kernels::flashinfer_decode_attention` as the missing
wiring. That kernel is real (already used by the CPU-testable reference
test `flashinfer_kernel_dependency_runs_reference_decode_attention`), and —
unlike paged attention's `flash_attn_varlen_paged_windowed` — it has no
F16/BF16-only restriction (F32/F16/BF16 all supported, plus a genuine CPU
fallback) and expects K/V in exactly the shape Tachyon's existing contiguous
KV cache already uses. This makes it a smaller, lower-risk follow-up to
land now that `wire-paged-attention-decode-path` (#341/#342) proved the
overall pattern (fork seam → block/attention wiring → real-GPU proof) works.

## What Changes

- Files an additive seam in the `astorise/candle` fork (out of this repo's
  tree, tracked as an external prerequisite — see Impact) analogous to
  `Cache::set_paged_kv`: a `use_flashinfer_attention` flag threaded through
  `Config`/`CausalSelfAttention`, taking effect only at the decode step
  (one query token per sequence — `flashinfer_decode_attention` is not a
  prefill kernel) and reusing the existing contiguous `Cache.kvs[block_idx]`
  storage untouched otherwise.
- Once that lands: lifts `flashinfer_attention`'s rejection for a Llama
  checkpoint on CUDA (mirroring the same architecture/device gate
  `paged_attention` and the single-device CUDA baseline already use); every
  other architecture/device/build combination keeps the existing typed
  rejection.
- No dtype switch needed (unlike paged attention's BF16 requirement) and no
  new block-allocator state — this reuses the existing contiguous KV cache,
  so the diff is expected to be much smaller than `wire-paged-attention-decode-path`.
- Adds a GPU CI proof (`cuda-quality`) exercising a real
  `flashinfer_attention: true` generation, following the same pattern as
  the paged-attention proof (and expecting a similar real-hardware
  debugging cycle for kernel-contract details not visible from CPU
  compilation — paged attention needed three).

## Capabilities

### New Capabilities
(none — flashinfer_attention is already a documented capability of
`ai-inference`, currently fail-closed)

### Modified Capabilities
- `ai-inference`: the "CUDA Graph and FlashInfer decode acceleration MUST
  be explicit and fail-closed" requirement's FlashInfer half changes from
  an unconditional rejection to a real decode-attention path for
  Llama-family CUDA deployments, with fail-closed rejection preserved for
  every other architecture/device/build combination. (`cuda_graph_decode`
  is untouched by this change — still rejected, tracked separately.)

## Impact

- `core-host/src/ai_inference/candle_llm_runtime.rs`: the
  `flashinfer_attention` rejection branch in `try_load_with_topology`
  becomes architecture/device-aware; the Llama decode path gains a way to
  request the flashinfer-backed attention kernel per layer.
- `core-host/Cargo.toml`: bumps the pinned `astorise/candle` tag once the
  fork seam lands (the `candle-flashinfer`/`candle-cuda` features already
  exist from prior work — see `core-host/Cargo.toml`'s `candle-flashinfer`
  feature).
- `openspec/specs/ai-inference/spec.md`: requirement + scenario updates
  described above.
- `.github/workflows/ci.yml`: new `cuda-quality` step for the
  flashinfer-attention proof.
- **External dependency**: [astorise/candle#11](https://github.com/astorise/candle/issues/11)
  (filed, not yet resolved as of this proposal) — an additive
  `use_flashinfer_attention` seam in `candle-transformers::models::llama`.
  Tachyon-Mesh wiring tasks are blocked until that tag lands, same pattern
  as `enable-single-device-llama-cuda-execution`/`wire-paged-attention-decode-path`.
- Out of scope here: `cuda_graph_decode` (needs a materially more invasive
  fork change — device-tensor-based rotary position handling instead of
  the current host-side `narrow`, to make the decode step genuinely
  CUDA-graph-replayable across incrementing positions — tracked as a
  separate, harder follow-up) and continuous batching (no fork dependency,
  but a substantial Tachyon-Mesh scheduling change, also tracked separately).
