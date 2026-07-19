## Why

Issue #312 step 2: `hardware_strategy.cuda_graph_decode` is fully plumbed
through manifest, schema, UI, and MCP, but `CandleLlmRuntime::try_load_with_topology`
rejects every request for it unconditionally, naming `candle_core::CudaGraph`
as the missing wiring. `CudaGraph::capture`/`replay` are real in the pinned
fork (`candle-core/src/cuda_backend/graph.rs`), but making Tachyon's decode
step genuinely replayable across incrementing positions requires solving a
real correctness problem this proposal's design.md documents precisely,
not just "wire the API up."

## What Changes

- Files an additive seam in the `astorise/candle` fork (external
  prerequisite, tracked separately — see Impact): a persistent,
  in-place-updatable device tensor driving rotary-embedding position lookup
  (`index_select`/gather) instead of the current host-side
  `cache.cos.narrow(0, index_pos, seq_len)`, which bakes a
  capture-time-only offset into any captured graph and would silently read
  stale rotary embeddings on replay at a different position otherwise.
- **Hard scoping discovery**: `cuda_graph_decode` can only compose with
  `hardware_strategy.paged_attention` on the Tachyon-Mesh side, not the
  contiguous KV cache — the contiguous cache grows via `Tensor::cat` (a
  fresh, larger, differently-shaped allocation every decode step), which
  `CudaGraph::capture`'s own docs disallow ("a tensor allocated inside `f`
  becomes a graph-owned allocation node"). Paged attention's `PagedKvCache`
  (pre-allocated once, written in place via `scatter_set`) already has the
  right shape for graph capture. This proposal requires `paged_attention:
  true` whenever `cuda_graph_decode: true` is requested, rather than
  attempting to make the contiguous path graph-safe too.
- Once the fork seam lands: wires a capture/replay decode loop for a
  paged-attention Llama deployment on CUDA — a warm-up (uncaptured) decode
  step, then `CudaGraph::capture` around the steady-state single-token
  step, replayed for subsequent tokens with only the input-token buffer,
  decode-position tensor, and paged block-table/seqlens tensors updated in
  place (not reallocated) between replays. Recaptures when a sequence
  crosses a page-block boundary changes the block-table shape (or accepts
  a fixed maximum block-table width sized upfront, avoiding recapture —
  design.md picks one).
- Every other combination (no `paged_attention`, non-Llama architecture,
  non-CUDA device, build predating the fork seam) keeps the existing
  typed rejection.

## Capabilities

### New Capabilities
(none — `cuda_graph_decode` is already a documented capability of
`ai-inference`, currently fail-closed)

### Modified Capabilities
- `ai-inference`: the "CUDA Graph and FlashInfer decode acceleration MUST
  be explicit and fail-closed" requirement's CUDA Graph half changes from
  an unconditional rejection to a real capture/replay decode path for
  paged-attention Llama-family CUDA deployments, with fail-closed rejection
  preserved for every other combination — including a paged-attention
  deployment that does *not* also request `cuda_graph_decode` (unaffected)
  and a `cuda_graph_decode` request *without* `paged_attention` (still
  rejected, now with a detail naming the dependency).

## Impact

- `core-host/src/ai_inference/candle_llm_runtime.rs`: the
  `cuda_graph_decode` rejection branch becomes architecture/device/
  paged-attention-dependency-aware; `paged_llama_forward` (or a sibling)
  gains a capture/replay mode.
- `core-host/Cargo.toml`: bumps the pinned `astorise/candle` tag once the
  fork seam lands.
- `openspec/specs/ai-inference/spec.md`: requirement + scenario updates.
- `.github/workflows/ci.yml`: new `cuda-quality` step for the
  cuda-graph-decode proof.
- **External dependency**: [astorise/candle#12](https://github.com/astorise/candle/issues/12)
  (filed, not yet resolved as of this proposal) — a `Cache::set_decode_position`-style
  seam. Tachyon-Mesh wiring tasks are blocked until that lands, same
  pattern as the two prior changes in this series.
- **Internal dependency**: builds on `wire-paged-attention-decode-path`
  (#341/#342, already merged) — specifically its `PagedAttentionRuntime`/
  `PagedSequenceGuard`/`paged_llama_forward` — this change extends rather
  than replaces that machinery.
- Out of scope here: continuous batching (issue #312 step 4 — no fork
  dependency, but its own substantial Tachyon-Mesh scheduling change,
  tracked separately, and expected to land *before* this change in
  practice given `cuda_graph_decode` is expected to be the hardest/last
  of the three remaining items in this series).
