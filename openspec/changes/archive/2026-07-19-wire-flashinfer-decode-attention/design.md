## Context

`hardware_strategy.flashinfer_attention` is rejected unconditionally today
(`candle_llm_runtime.rs`, alongside the `paged_attention`/`cuda_graph_decode`
rejections it originally shared a code block with before
`wire-paged-attention-decode-path` split `paged_attention`'s gate out). The
pinned `astorise/candle` fork already implements the kernel:

```rust
/// Single-token decode attention: softmax(q @ k^T * softmax_scale) @ v, where
/// each sequence in the batch contributes exactly one query token attending
/// over its key/value cache. Grouped-query attention is supported.
/// q: (batch, num_heads_q, head_dim)
/// k, v: (batch, num_heads_kv, seqlen_k, head_dim)
/// -> (batch, num_heads_q, head_dim)
pub fn flashinfer_decode_attention(q: &Tensor, k: &Tensor, v: &Tensor, softmax_scale: f32) -> Result<Tensor>
```

Two things make this materially simpler than `wire-paged-attention-decode-path`:
- **No dtype restriction**: the CUDA dispatch handles F32/F16/BF16
  (`"flashinfer-decode-attention is only supported for f32/f16/bf16"` only
  rejects other dtypes), and there's a real CPU fallback too. No BF16
  switch needed — a flashinfer-attention Llama deployment stays F32, same
  as the plain dense path.
- **No new KV storage**: `k`/`v` are expected as
  `(batch, num_heads_kv, seqlen_k, head_dim)` — exactly the shape
  `candle_transformers::models::llama::Cache.kvs[block_idx]` (the existing
  contiguous cache) already stores. No block allocator, no external tensor
  ownership.

The gap is the same shape as paged attention's, though: Tachyon's runtime
doesn't run its own transformer loop, and `CausalSelfAttention::forward`
(where the decode-step attention dispatch — `use_flash_attn ? flash_attn(..)
: dense_matmul_softmax(..)` — lives) is private. There is no way for an
external crate to substitute `flashinfer_decode_attention` into that
dispatch without a fork-side seam.

## Goals / Non-Goals

**Goals:**
- Real `flashinfer_decode_attention` execution for the decode step (single
  query token per sequence) of a Llama checkpoint on CUDA, replacing the
  dense matmul+softmax (or `flash_attn`) attention computation for that
  step only.
- Keep prefill (multi-token forward) on the existing path unconditionally —
  `flashinfer_decode_attention` is a decode-only kernel by design.
- No dtype change: a flashinfer-attention deployment stays on the same F32
  path as the plain dense Llama path (contrast with paged attention's BF16
  requirement).
- Every other architecture, non-CUDA device, or build without the fork
  seam keeps the existing typed rejection, byte-for-byte.

**Non-Goals:**
- `cuda_graph_decode` — untouched, still rejected. Tracked separately;
  materially harder (needs device-tensor-based rotary position handling to
  make the decode step CUDA-graph-replayable across incrementing
  positions, since the current `cache.cos.narrow(0, index_pos, seq_len)`
  bakes a host-computed offset into the captured graph — replaying at a
  different position would silently read the wrong rotary embeddings).
- Continuous batching — no fork dependency, but a separate, substantial
  Tachyon-Mesh scheduling change, out of scope here.
- Combining `flashinfer_attention` with `paged_attention` in the same
  deployment — the fork's `flashinfer_decode_attention` operates on the
  contiguous cache shape, not the paged block layout; composing both would
  need its own design. Reject the combination with a typed error rather
  than silently picking one.

## Decisions

### 1. Required `astorise/candle` seam (external prerequisite)
Filed as [astorise/candle#8](https://github.com/astorise/candle/issues/8)'s
sibling, [astorise/candle#11](https://github.com/astorise/candle/issues/11).
Proposed shape: an additive `use_flashinfer_attention: bool` on `Config`
(mirroring the existing `use_flash_attn`), and a new branch in
`CausalSelfAttention::forward` — after the existing contiguous-cache update
and *before* `repeat_kv` (since `flashinfer_decode_attention` handles GQA
internally, unlike the dense/flash_attn paths which need heads
pre-repeated) — that calls `flashinfer_decode_attention(q, k, v,
softmax_scale)` when `use_flashinfer_attention` is set **and** `seq_len ==
1`. Prefill (`seq_len > 1`) always uses the existing path regardless of the
flag. Exact tensor-shape bookkeeping (squeeze/reshape to match the
kernel's expected 3D `q`) is left to whoever implements the fork PR — the
proposal is illustrative, not a literal patch, same as how #8's actual
implementation adjusted the originally-proposed `Cache::Paged(..)` enum to
a simpler per-layer-slot design once someone worked through the real
constraints.

Alternative considered: extend the existing `use_flash_attn` boolean into a
3-way enum (`Dense`, `FlashAttn`, `FlashInfer`) instead of a second
boolean. Deferred to whoever implements the fork PR to decide — either is
additive and doesn't change default behavior; not worth prescribing from
outside the fork.

### 2. Feature gating and rollout (same pattern as paged attention)
- `flashinfer_attention` continues to be rejected with the existing typed
  error for: non-Llama architectures, non-CUDA devices, and any build
  where the pinned fork tag predates the seam.
- Once the fork tag bump lands, the Llama+CUDA branch in
  `try_load_with_topology` builds the decode-attention hook instead of
  erroring, gated behind the existing `candle-flashinfer` Cargo feature
  (already present from prior work, currently only exercised by the
  reference-kernel test).
- Unlike paged attention, no dtype switch and no per-layer external tensor
  allocation — the wiring is expected to be a small addition to the
  existing Llama load/decode path, not a new subsystem.

### 3. Testability
The reference kernel call (`flashinfer_decode_attention` on fixed
tensors) already has a CPU-runnable test today
(`flashinfer_kernel_dependency_runs_reference_decode_attention`, gated on
the `candle-flashinfer` feature only, not `candle-cuda` — it exercises the
CPU fallback). The actual Llama-integrated generation test still needs
`candle-cuda` + a real GPU, following the same `cuda-quality` pattern as
paged attention. Given paged attention needed three real-hardware
iterations to catch kernel-contract details invisible from CPU compilation
(dtype, block-size alignment, head-dim alignment), budget for a similar
number of iterations here even though this kernel has fewer known
constraints (no block size, no head-dim-multiple-of-8 requirement
documented in its source — but that should be verified against the actual
kernel source before assuming it, not assumed from this design doc alone).

## Risks / Trade-offs

- **[Risk] This change depends on work in a different repository
  (`astorise/candle`) this OpenSpec change cannot implement or merge
  directly.** → Mitigation: tracked as an explicit external prerequisite
  ([astorise/candle#11](https://github.com/astorise/candle/issues/11)),
  same pattern as the two prior changes in this series.
- **[Risk] Unknown kernel-contract constraints** (paged attention's
  `page_block_size % 32` and `head_dim % 8` requirements were only
  discovered via real `cuda-quality` runs, not from reading the Rust
  binding alone) — checked this time *before* implementing, unlike paged
  attention: `candle-flashinfer-kernels/kernels/decode_attention.cu` is a
  reference-style kernel (`grid = (batch, num_heads)`, `block =
  next_pow2(head_dim)` capped at 1024, shared-memory reduction with
  `tid < head_dim` bounds checks) with **no block-size or head-dim
  divisibility requirement** — unlike the tiled, tensor-core-oriented
  flash-attn kernel. The only real constraints found in the Rust binding
  (`candle-flashinfer-kernels/src/lib.rs`) are: `num_heads_k` must divide
  `num_heads_q` (standard GQA, already guaranteed by any valid Llama
  config), the head-dimension stride of `q`/`k`/`v` must be contiguous
  (satisfied by calling `.contiguous()` before the call, standard
  practice), and dtype must be f32/f16/bf16. → Mitigation: still budget
  for at least one real-GPU debugging round-trip regardless — a permissive
  kernel contract doesn't guarantee the *integration* (tensor shapes,
  strides, the fork's own new branch) is bug-free on the first real run,
  only that this specific class of prior surprise is less likely to repeat.
- **[Trade-off] Scoping to Llama only, decode-step only, no
  paged-attention composition** — matches the incremental-Llama-first
  pattern of every recent change in this series; broadening is a follow-up.
