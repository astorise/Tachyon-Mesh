## Context

`hardware_strategy.cuda_graph_decode` is rejected unconditionally today.
`candle_core::CudaGraph::capture`/`replay` are real in the pinned fork, with
an explicit contract in their own doc comment: captured operations' shapes
and buffer *addresses* must stay identical across replays, since a CUDA
graph records exact kernel launches, not a re-derivable computation.

Two parts of Tachyon's current decode path violate that contract:

1. **Rotary embeddings**: `cache.cos.narrow(0, index_pos, seq_len)` computes
   a host-side (Rust `usize`) offset into the precomputed cos/sin table,
   baked into the kernel launch that consumes the narrowed slice. Since
   `index_pos` increments every decode step, a graph captured at one
   position would replay against the *same, now-stale* memory location at
   every later step — a silent correctness bug, not just a missed
   optimization.
2. **Contiguous KV cache growth**: `Cache.kvs[block_idx]` grows via
   `Tensor::cat` — a new, larger, differently-shaped allocation every
   step. `CudaGraph::capture`'s docs explicitly disallow allocating inside
   the captured closure (`"a tensor allocated inside f becomes a
   graph-owned allocation node whose device memory is only valid while the
   graph is executing"`).

`wire-paged-attention-decode-path` (#341/#342, merged) already solves
problem 2 for the paged path specifically: `PagedKvCache`'s
`key_cache`/`value_cache` are pre-allocated once at load time and written
in place via `scatter_set`. It also, as a side effect of the OOM bug fix
documented there, caps the block pool at exactly `min_blocks` (one
full-length sequence's worth) — meaning a single sequence's block table
*already* has a known, fixed maximum width from the moment the model
loads, not something that grows unboundedly.

Problem 1 has no existing fix — it needs a fork-side seam
([astorise/candle#12](https://github.com/astorise/candle/issues/12)).

## Goals / Non-Goals

**Goals:**
- Real `CudaGraph` capture/replay for the steady-state decode step
  (post-prefill, one token at a time) of a **paged-attention** Llama
  deployment on CUDA.
- Every other combination (no `paged_attention`, non-Llama, non-CUDA,
  build predating the fork seam) keeps the existing typed rejection.
- No recapture during a single generation request: since the paged block
  pool is already capped at `min_blocks`, size the block-table/seqlens
  tensors at their full maximum width from the first decode step, so their
  *shape* never changes for the lifetime of the graph — only their
  *contents* do, via in-place writes, matching the graph-capture contract.

**Non-Goals:**
- Making the contiguous (non-paged) KV cache graph-capture-safe. Out of
  scope — `cuda_graph_decode` requires `paged_attention: true`.
- `flashinfer_attention` + `cuda_graph_decode` combined. Not addressed here;
  revisit once both land independently.
- Continuous batching. No interaction assumed yet — batch size 1 (today's
  scope for paged attention) is what gets captured; multi-sequence batches
  would need their own capture per batch-size bucket, a follow-up.

## Decisions

### 1. Required `astorise/candle` seam (external prerequisite)
Filed as [astorise/candle#12](https://github.com/astorise/candle/issues/12).
Proposed shape: `Cache::set_decode_position(&mut self, block_idx: usize,
position: Tensor)`, a persistent per-layer device tensor the model's
rotary-embedding application reads via `index_select`/gather instead of
`narrow(0, index_pos, seq_len)` whenever attached. The caller
(Tachyon-Mesh) updates this tensor's *contents* in place before each
replay; its address must not change.

### 2. `cuda_graph_decode` requires `paged_attention`
Rather than attempting to make the contiguous cache graph-safe too (a
second, independent hard problem), this change requires
`hardware_strategy.paged_attention: true` whenever `cuda_graph_decode:
true` is set, and rejects `cuda_graph_decode` without it with a typed
error naming the dependency. This is a real architectural constraint, not
an arbitrary restriction — see Context.

### 3. Fixed-width block-table/seqlens tensors from the first decode step
Because `PagedBlockPool` is already capped at `min_blocks` (one
full-length sequence, per `wire-paged-attention-decode-path`'s OOM-bug
fix), a single sequence's block table never needs more than `min_blocks`
columns for the lifetime of a request. Instead of `build_block_table_tensor`/
`build_cumulative_seqlens_tensor` allocating a **new** tensor sized to the
*currently used* block count on every call (today's behavior, fine for the
non-captured path but graph-unsafe), the captured decode path pre-allocates
`(1, min_blocks)` and `(2,)` tensors once, zero-padded, and writes into them
in place (`Tensor::slice_set`/`scatter_set`) as the sequence grows — shape
is constant from step 1, so no recapture is ever needed within a request.

Alternative considered: recapture whenever the sequence crosses a
page-block boundary (variable-width tensors, like the existing
non-captured helpers already do). Rejected — recapture cost likely
dominates any steady-state replay savings for short sequences, and the
fixed-width approach is already fully supported by the existing
`min_blocks`-capped pool with no additional memory cost (the padding slots
were already implicitly reserved by the pool's fixed sizing).

### 4. Capture/replay orchestration
Per request: run the existing (uncaptured) prefill as today; on the first
decode step, do one *warm-up* call (uncaptured, per `CudaGraph::capture`'s
requirement to JIT-load kernels/populate the host-to-device upload cache
before capture), then call `CudaGraph::capture` around a second logical
call of the same operations, keeping the resulting `CudaGraph` for the
rest of the request; every subsequent decode step writes the new token id,
position, and (if grown) block-table/seqlens contents into the persistent
buffers, then calls `graph.replay()` instead of running the closure again.
The graph (and its buffers) are scoped to one request/`PagedSequenceGuard`
lifetime — not reused across requests, at least initially (a longer-lived,
cross-request graph is a further optimization, not attempted here).

### 5. NEW BLOCKER (found 2026-07-10, before any capture/replay code was written): `PagedKvCache::write_new_kv` is not graph-capturable

Discovered by reading the fork's actual `write_new_kv` implementation
(`candle-transformers/src/models/llama.rs`, tag `tachyon-v0.11.0-4`) while
starting Section 2, before attempting any capture: every forward pass through
the paged path calls `write_new_kv(k, v, index_pos)`, whose *first* operation
is

```rust
let block_table = self.block_table.to_dtype(DType::U32)?.to_vec2::<u32>()?;
```

`Tensor::to_vec2` on a CUDA tensor is a blocking device-to-host copy —
exactly the kind of host/device synchronization `CudaGraph::capture`'s own
contract disallows mid-capture (CUDA's stream-capture API rejects
synchronizing operations issued on a capturing stream). `write_new_kv` then
builds `slots: Vec<u32>` on the host from that readback and uploads it via
`Tensor::from_vec(slots, ...)` — a *new* device allocation created inside the
call, which independently violates the "no allocation inside the captured
closure" rule Decision 3 already accounts for elsewhere.

This means the premise in this design's Context section — "paged attention's
`PagedKvCache` already has the right shape for graph capture" — is only true
for the pre-allocated `key_cache`/`value_cache` buffers themselves, not for
how `write_new_kv` computes where to scatter into them. Attempting to capture
a decode step through the existing paged path as-is would either fail
outright (CUDA stream-capture error) or, if some layer of the stack tolerated
it, silently freeze the *destination* indices to whatever `index_pos` produced
at capture time while still using fresh `k`/`v` *values* on replay — the same
class of silent-staleness bug this change's whole premise (`Cache::set_decode_position`)
already exists to prevent for rotary embeddings, just relocated to KV
placement instead.

Section 2's capture/replay orchestration (Decision 4) cannot proceed as
designed until this is resolved. The natural fix mirrors `set_decode_position`'s
shape: either (a) an additive `write_new_kv` variant that computes scatter
indices via on-device tensor arithmetic (`block_table.gather(...)` combined
with a device-side offset, no host readback), or (b) letting the caller
supply pre-computed device indices directly, since Tachyon's own
`SequenceBlockTable` already knows the block assignment on the host before
ever uploading `block_table` — the fork doesn't need to re-derive it via a
round-trip. Not yet filed upstream; this is a **new, fourth** fork
dependency this change did not originally anticipate, on top of
`Cache::set_decode_position` (already landed). Until it lands, `cuda_graph_decode`
remains rejected — this change's Section 2 gate work (requiring `paged_attention`,
rejecting every other combination) is implemented and real, but the actual
capture/replay orchestration is not, and should not be attempted against the
current `write_new_kv` implementation.

## Risks / Trade-offs

- **[Risk] This change depends on work in a different repository this
  OpenSpec change cannot implement or merge directly.** → Mitigation:
  tracked as an explicit external prerequisite
  ([astorise/candle#12](https://github.com/astorise/candle/issues/12)),
  same pattern as the two prior changes in this series.
- **[Risk, materialized] A second, unanticipated fork dependency was found
  while starting Section 2** (Decision 5 above): `PagedKvCache::write_new_kv`'s
  host-side block-table readback is not graph-capturable, independent of the
  rotary-embedding problem `Cache::set_decode_position` already solves. This
  was caught by reading the fork's source carefully *before* writing any
  capture/replay code, rather than via a failed `cuda-quality` run — cheaper
  than the alternative, but it means Section 2's actual orchestration work
  cannot start until a fourth fork seam lands. → Mitigation: documented
  precisely here so whoever files that issue (or picks this back up) doesn't
  need to rediscover it; the gate/rejection work (Section 2's device/arch/
  paged_attention-dependency checks) already merged independent of this,
  since it doesn't touch capture/replay at all.
- **[Risk] This is the most invasive of the three remaining #312 items** —
  it touches rotary-embedding internals (fork-side) and requires
  reworking the paged decode path's tensor lifetime management (buffer
  reuse instead of per-call allocation) on the Tachyon-Mesh side.
  Consistent with paged attention needing three real-hardware debugging
  iterations for a comparatively simpler seam, budget for several here too
  — and budget the *design* effort (this document) as real, non-trivial
  work in its own right, which is why it was scoped and written before
  any implementation began.
- **[Trade-off] Only batch size 1, no cross-request graph reuse, no
  recapture-on-error handling designed yet** — matches the "one sequence
  in flight" scope every change in this series has kept so far;
  continuous batching (issue #312 step 4) would need its own design pass
  on top of this one, not the reverse — this is part of why continuous
  batching is expected to land *before* `cuda_graph_decode` in practice.
