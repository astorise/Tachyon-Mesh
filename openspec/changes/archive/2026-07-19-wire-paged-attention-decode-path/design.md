## Context

`hardware_strategy.paged_attention` is fully plumbed through manifest,
schema, UI, and MCP, but `CandleLlmRuntime::try_load_with_topology` rejects
every request for it unconditionally
(`core-host/src/ai_inference/candle_llm_runtime.rs:1003-1010`), matching the
current `ai-inference` spec's fail-closed requirement. The pinned
`astorise/candle` fork (`tachyon-v0.11.0-1` at proposal time, now
`tachyon-v0.11.0-3`) already implements the CUDA kernel this needs —
`candle_flash_attn::flash_attn_varlen_paged_windowed(q, k, v, seqlens_q,
seqlens_k, block_table, mm_prefix_ranges, max_seqlen_q, max_seqlen_k,
softmax_scale, window_size_left, window_size_right, page_block_size,
softcap)` — with `k`/`v` laid out as `(num_blocks, page_block_size,
num_heads_kv, head_size)` and `block_table` as `(batch_size, max_blocks)`
physical block indices per sequence: a standard vLLM-style paged KV layout.
It is CUDA-only (`candle-flash-attn` calls into `CudaStorage` directly, no
CPU fallback), so this only ever applies to CUDA deployments.

The gap is entirely on the model-forward side. Tachyon's runtime does not
run its own transformer loop — it calls
`candle_transformers::models::llama::Llama::forward`, whose `CausalSelfAttention`
concatenates the new K/V onto a contiguous per-request cache
(`Cache.kvs[block_idx]`) every step
(`candle-transformers/src/models/llama.rs:396-450`). `CausalSelfAttention`,
`Mlp`, `Block`, and their `forward` methods are private
(not `pub`) to that module, and `Cache` has no seam for external paged
storage. There is no way for an out-of-crate caller to substitute paged
attention into an existing `Llama::forward` call today.

## Goals / Non-Goals

**Goals:**
- Give the Llama family (the architecture Tachyon's NVFP4/adapter/speculative
  work already concentrates on) a real, block-paged decode path on CUDA when
  `hardware_strategy.paged_attention: true`.
- Own the block allocator and per-sequence block table in Tachyon-Mesh (not
  the fork), since block lifetime is a Tachyon scheduling concern (admission,
  eviction, sharing across requests) — the fork should only own the kernel
  call and the tensor layout it expects.
- Propose the minimal, additive `pub` seam `astorise/candle` needs so
  Tachyon-Mesh can drive that kernel without duplicating the whole Llama
  transformer stack.
- Keep every other architecture, every non-CUDA device, and
  `paged_attention: false` byte-for-byte unchanged (existing contiguous path,
  existing rejection error for what's still unsupported).

**Non-Goals:**
- `cuda_graph_decode`, `flashinfer_attention`, and continuous batching —
  separate follow-up changes (continuous batching depends on this one's
  block table).
- Non-Llama architectures. They keep returning the existing
  `UnsupportedModel` rejection for `paged_attention` until they get the same
  treatment in a later change.
- Cross-node paged KV (block tables are single-node/single-process for now,
  matching the existing single-node KV cache model).
- Implementing or merging the `astorise/candle` PR itself. That repository
  is out of this OpenSpec change's tree; this design only specifies the API
  shape Tachyon-Mesh needs from it.

## Decisions

### 1. Block allocator and block table live in `core-host`, not the fork
A fixed-size block pool (`PagedBlockPool`) sized at load time from
`hardware_strategy` (or a runtime default) and gpu memory budget:
- `page_block_size` tokens per block (constant per deployment, matches the
  kernel's `page_block_size` argument).
- A free-list of physical block ids; `allocate_block()` /
  `free_blocks(&[BlockId])`.
- `SequenceBlockTable { blocks: Vec<BlockId> }` per in-flight sequence,
  growing by one block every `page_block_size` tokens.
- At each decode step, the runtime builds the `(batch_size, max_blocks)`
  `block_table` tensor (physical ids, padded) and `seqlens_k` from every
  active sequence's table, exactly matching
  `flash_attn_varlen_paged_windowed`'s expected inputs.

Alternative considered: let the fork own block allocation (like vLLM's
`BlockManager` living inside the inference engine). Rejected — Tachyon
already treats scheduling/admission as host-runtime concerns (see the
continuous-batching follow-up), and keeping allocation in `core-host` means
the fork stays a thin, reusable kernel/primitive layer, consistent with why
`CudaGraph` and `flashinfer_decode_attention` were added there as primitives
rather than as scheduling policy.

### 2. `astorise/candle` seam — delivered on tag `tachyon-v0.11.0-3`
Filed as [astorise/candle#8](https://github.com/astorise/candle/issues/8) and
landed on tag `tachyon-v0.11.0-3`. The delivered shape keeps `Cache` a
struct (not the enum this design originally proposed), adding a per-layer
paged slot instead:

```rust
// candle-transformers::models::llama
pub struct PagedKvCache {
    pub key_cache: Tensor,   // (num_blocks, page_block_size, num_kv_heads, head_dim)
    pub value_cache: Tensor, // (num_blocks, page_block_size, num_kv_heads, head_dim)
    pub block_table: Tensor, // (batch_size, max_blocks), physical block ids
    pub seqlens_k: Tensor,   // (batch_size + 1,), cumulative
    pub page_block_size: usize,
}

impl Cache {
    pub fn set_paged_kv(&mut self, block_idx: usize, paged: PagedKvCache) -> Result<()>;
    pub fn clear_paged_kv(&mut self, block_idx: usize);
    pub fn paged_kv(&self, block_idx: usize) -> Option<&PagedKvCache>;
}
```

`CausalSelfAttention::forward` checks `cache.paged_kv(block_idx)` first: if
set, it writes the current step's K/V into the caller-owned
`key_cache`/`value_cache` at the slots `block_table` designates (host-computed
slot indices + `Tensor::scatter_set`) and calls the CUDA-only
`flash_attn_varlen_paged_windowed` (gated behind candle-transformers'
`flash-attn` feature, which requires `cuda`); with no paged cache attached it
falls back to today's contiguous concat-and-narrow path byte-for-byte. This
kept `CausalSelfAttention`/`Block`/`Mlp` private — no existing call site
(including `candle-examples`) needed to change. `block_table`/`seqlens_k`
are read-only from the model's side: `core-host` owns allocation, eviction,
and keeping them in sync, matching Decision 1 below.

Alternative considered (and rejected, matching the original proposal):
duplicate `Llama`/`Block`/`CausalSelfAttention` inside `core-host` instead of
extending the fork — would have doubled the maintenance surface for every
future Llama-family fix (RoPE, GQA `repeat_kv`, LoRA hooks). The delivered
per-layer-slot shape achieves the same additive, zero-regression goal as the
originally proposed enum with a smaller diff against upstream `candle-transformers`.

### 3. Feature gating and rollout
- `hardware_strategy.paged_attention` continues to be rejected with the
  existing typed error for: non-Llama architectures, non-CUDA devices, and
  any build where the pinned fork tag predates the `PagedKvCache` seam.
- Once the fork tag bump lands, `core-host/Cargo.toml`'s pinned tag moves
  forward and the Llama+CUDA branch in `try_load_with_topology` builds the
  block pool/table and passes `Cache::Paged(..)` into the load path instead
  of erroring.
- This is only reachable under the existing `candle-cuda` feature (paged
  flash-attn has no CPU implementation) — CPU builds keep compiling and
  keep returning the existing error text.

### 4. Testability
`PagedBlockPool`/`SequenceBlockTable` allocate/free/grow logic is pure Rust
and unit-testable on CPU (allocation exhaustion, block reuse after free,
block-table tensor construction for a batch of sequences at various
lengths) regardless of GPU availability.

The actual `flash_attn_varlen_paged_windowed` call and a real generation
through `cache.set_paged_kv(..)` need a CUDA device with the `candle-cuda`
feature. This dev machine has a real GPU (RTX 3070, 8GB) and the CUDA 13.2
toolkit, but `cargo check -p core-host --features candle-cuda` does not
currently compile here: `nvcc` fails every kernel with `Host compiler
targets unsupported OS` against the installed MSVC toolset (14.39.33523,
under a Visual Studio "18" install with no `vcvarsall.bat` present) —
reproduces identically from both Git Bash and native PowerShell, so it is a
toolchain/version-pairing issue, not a shell-environment artifact. This is
an environment gap to fix separately (older MSVC toolset, or a CUDA
toolkit release that recognizes this one), not a code issue. Until it's
resolved, `cuda-quality` (`arc-gpu-runners`, Linux) remains the only lane
that actually compiles and runs the CUDA-gated code below, same as
originally assumed before this GPU was discovered.
`paged_attention_strategy_is_rejected_until_block_tables_are_wired` is scoped
down to the cases that still reject (non-Llama architecture, non-CUDA
device); a new CUDA-gated test covers the now-enabled Llama+CUDA case.

### 5. Paged attention requires BF16, not F32 (discovered starting Section 3)
`candle-flash-attn`'s CUDA dispatch is exhaustive over exactly two dtypes —
`match q.dtype() { F16 => .., BF16 => .., dt => bail!("flash-attn is only
supported for f16/bf16 ({dt:?})") }` — every kernel file in the crate is
named `*_fp16_*`/`*_bf16_*`, none `*_fp32_*`. The single-device Llama path
(`VarBuilder`, `Cache`) loads/computes in F32 unconditionally today. This
means enabling paged attention isn't just "attach a block table" — it
requires switching the model's working dtype for a paged deployment, the
same constraint every real paged-attention implementation has (vLLM,
TensorRT-LLM also require FP16/BF16 for their fused attention kernels; this
isn't a corner cut specific to Tachyon).

`load_safetensors` now selects `DType::BF16` (over `DType::F16`, for its
wider exponent range / better numerical stability for LLM inference,
matching modern industry default) for the `VarBuilder` and `Cache` only
when `architecture == Llama && strategy.paged_attention`; every other
combination stays `F32`, byte-for-byte unchanged. `Cache::new`'s rotary
cos/sin precompute and `candle_nn::rotary_emb::rope` already parametrize
over `dtype` generically (no upstream change needed for that part — only
the new `PagedKvCache` seam itself required a fork change). The forward
closure that attaches `cache.set_paged_kv` converts the model's BF16 logits
back to `F32` before they reach the shared sampling/FSM-masking pipeline,
which assumes `F32` throughout (`mask_row_for_fsm`'s `to_vec1::<f32>()`
would otherwise error on a BF16 tensor).

**Consequence for the "matches the non-paged path's output" test goal**:
BF16 vs. F32 will not produce bit-identical logits (precision loss
compounds over decode steps), so Task 4.2's test proves a real, non-empty,
*deterministic* decode (repeating the same greedy request against the
shared block pool yields identical output) rather than exact numerical
parity with the F32 dense path — the same relaxation the NVFP4 dequantized
forward-pass test already accepted for a different precision trade-off.

## Newly discovered blocker (found while starting implementation)

Section 3's premise — "attach paged KV to a Llama binding on a CUDA device"
— assumed the single-device (`GpuDistribution::Single`) Llama path could
already reach a CUDA device, based on a comment at
`candle_llm_runtime.rs:1031-1036` attributing GPU dense execution to the
separate `gpu-accelerated-inference-execution` change. That change turned
out to only cover ONNX GPU dispatch and the NVFP4 CPU-dequant fallback (see
this change's own project-memory note); it never touched the plain Llama
safetensors/GGUF backend.

In fact, every single-device loader in `candle_llm_runtime.rs`
(`load_safetensors:1108`, `load_gguf:1443`, the NVFP4 loader:~1348, the
LoRA-adapter loader:~1843) hardcodes `let device = Device::Cpu;`
unconditionally, and `try_load_with_topology` (line 1037) rejects any
`distribution_mode: single` request for a non-`cpu` device *before* the
`paged_attention` check is ever reached. Only the tensor/pipeline/expert
**parallel** engines (`load_parallel`, `TensorParallelLlama`/
`PipelineParallelLlama`/`ExpertParallelLlama`) construct real CUDA devices —
and those use their own duplicated cache types (e.g. `TensorParallelCache`,
`tensor_parallel_llama.rs:36`, explicitly "functionally identical to
`candle_transformers::models::llama::Cache`, but ..."), not the plain
`candle_transformers::models::llama::Cache` the new `set_paged_kv`/`paged_kv`
seam was added to.

So the fork seam this change depends on is only reachable through a code
path (`single`-strategy Llama on CUDA) that does not exist yet in
Tachyon-Mesh, for any architecture, independent of paged attention. Wiring
`cache.set_paged_kv` onto `Llama::load`'s output cannot produce a runnable
path until the single-device loader can build a real CUDA device and
successfully run `Llama::forward` on it at all (dense, non-paged, as a
baseline). That is a materially larger and logically prior piece of work —
general single-device GPU dense execution — that this change did not
originally scope, and it would also be the same prerequisite
`cuda_graph_decode` and `flashinfer_attention` need later, since both are
single-device, GPU-only toggles behind the same `Device::Cpu` wall.

This is now tracked as a separate, narrower prerequisite change,
`enable-single-device-llama-cuda-execution` (see `tasks.md` section 0),
rather than folded silently into "wire the paged path" — building the
paged plumbing on top of a load path that can never reach CUDA would
produce code with no way to execute, which is worse than not writing it.

## Risks / Trade-offs

- **[Risk] This change depended on work in a different repository
  (`astorise/candle`) that this OpenSpec change could not implement or merge
  directly.** → Resolved: [astorise/candle#8](https://github.com/astorise/candle/issues/8)
  landed on tag `tachyon-v0.11.0-3` (see Decision 2). `tasks.md` tracked this
  as an explicit, separately-tracked prerequisite rather than a task this
  change claimed to complete before it existed, same pattern as
  `gpu-accelerated-inference-execution`'s "superseded/deferred" task notes.
- **[Risk] Paged KV storage changes the memory-sizing story** (a block pool
  reserves VRAM up front instead of growing per-request) — **this actually
  happened and OOM'd a real GPU.** The first `size_paged_kv_pool` sized the
  pool to whatever fit the *entire* free-VRAM budget (a fixed 50% fraction)
  and omitted `num_hidden_layers` from the per-block byte cost; for the tiny
  CI test fixture against a real GPU with gigabytes nominally free, that
  computed millions of blocks, and `single_device_llama_paged_attention_generates_a_real_decode_on_cuda`
  failed on `cuda-quality` with `CUDA_ERROR_OUT_OF_MEMORY` — caught only by
  the real hardware run, not by CPU compilation or unit tests. Fixed by
  capping the pool at exactly `min_blocks` (one full-length sequence; only
  one sequence is ever in flight per model until continuous batching lands)
  regardless of how much more VRAM is free, and including
  `num_hidden_layers` in the cost. The sizing arithmetic was also extracted
  into a pure `size_paged_kv_pool` function so this exact class of bug is
  now covered by CPU-only regression tests
  (`paged_attention_pool_is_capped_at_one_sequence_even_with_abundant_free_vram`)
  instead of only being discoverable on real GPU hardware.
- **[Risk] Hardcoded kernel constants can silently violate a real CUDA
  kernel's requirements** — **this also actually happened.** After fixing
  the OOM above, the very next `cuda-quality` run got past sizing but
  failed inside the forward pass itself:
  `candle_flash_attn`'s paged kernel hard-requires `page_block_size % 32 ==
  0`, and the original `PAGED_ATTENTION_PAGE_BLOCK_SIZE = 16` violated it —
  again, only discoverable on real hardware, since the CPU build never
  exercises the kernel at all. Fixed by changing the constant to `32` and
  adding a `const _: () = assert!(PAGED_ATTENTION_PAGE_BLOCK_SIZE.is_multiple_of(32),
  ..)` compile-time check, so reintroducing this exact value fails the
  build instead of needing another real-GPU round-trip to catch.
- **[Risk] Renovate cannot bump the fork automatically** (pinned by tag,
  flagged in the (now superseded) audit doc as a standing gap). →
  Mitigation: the tag bump for this change is manual and reviewed like the
  original fork pin; no change in that process is proposed here.
- **[Trade-off] Scoping to Llama only** leaves every other supported
  architecture still rejecting `paged_attention`. Accepted: matches how
  NVFP4 fallback and other recent runtime work incrementally covered Llama
  first; broadening is a follow-up once the fork seam and block-table
  plumbing are proven.
