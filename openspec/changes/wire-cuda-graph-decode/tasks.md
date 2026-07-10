## 1. External prerequisite (astorise/candle fork, separate repository)

- [x] 1.1 Land an additive `Cache::set_decode_position`-style seam in `candle-transformers::models::llama` (proposed shape in `design.md` Decision 1): a persistent per-layer device tensor the rotary-embedding application reads via `index_select`/gather instead of `narrow(0, index_pos, seq_len)` whenever attached, decode-step only. Tracked upstream as [astorise/candle#12](https://github.com/astorise/candle/issues/12), landed via [astorise/candle#14](https://github.com/astorise/candle/pull/14) — implementation matches the proposed shape (`Cache::set_decode_position(&mut self, block_idx, position)`/`clear_decode_position`/`decode_position`, `apply_rotary_emb` branches on it via `index_select`, errors if attached with `seq_len != 1`).
- [x] 1.2 Tag a new fork release once that PR merges and its own tests pass: `tachyon-v0.11.0-4` (same tag as `wire-flashinfer-decode-attention`'s prerequisite — both landed together).
- [x] 1.3 Bump `core-host/Cargo.toml`'s pinned `astorise/candle` tag to the new release (shared with `wire-flashinfer-decode-attention`).

## 2. Runtime wiring

- [x] 2.1 In `try_load_with_topology`, replace the unconditional `cuda_graph_decode` rejection with a gate requiring Llama + CUDA + `paged_attention: true` (design.md Decision 2); reject with a typed error naming the `paged_attention` dependency when it's missing, and keep the existing rejection for every other combination. **Implemented and real** — this is pure gate logic, independent of the capture/replay blocker below.
- [ ] 2.2 Change `build_paged_attention_runtime`'s per-request block-table/seqlens construction (or add a captured-mode variant) to pre-allocate `(1, min_blocks)`/`(2,)` tensors once per request and write into them in place (`slice_set`/`scatter_set`) as the sequence grows, instead of `build_block_table_tensor`/`build_cumulative_seqlens_tensor` allocating a fresh tensor sized to the currently-used block count on every call (design.md Decision 3). **Blocked** — see design.md Decision 5: doing this alone wouldn't help, since `write_new_kv`'s host readback breaks capture regardless of how the block-table tensor itself is (re)allocated.
- [ ] 2.3 Implement the capture/replay orchestration in `paged_llama_forward` (or a sibling): warm-up call, `CudaGraph::capture` on the first decode step, `graph.replay()` for subsequent steps, scoped to one request/`PagedSequenceGuard` lifetime (design.md Decision 4). **Blocked on a new, fourth fork dependency** found while starting this task (design.md Decision 5): `PagedKvCache::write_new_kv` computes scatter indices via a blocking device-to-host `Tensor::to_vec2` readback every forward pass — not capturable. Not yet filed upstream. Do not attempt capture against the current `write_new_kv` implementation; it will either fail outright or silently corrupt KV placement on replay.
- [ ] 2.4 Confirm LoRA adapters, speculative decoding, and prefix caching either compose or are explicitly rejected for a `cuda_graph_decode` deployment, following the same audit pattern as `wire-paged-attention-decode-path` Task 3.4. Deferred until 2.2/2.3 unblock — no capture/replay exists yet to audit against.

## 3. Tests

- [x] 3.1 `cuda_graph_decode_strategy_is_rejected_until_gpu_decode_is_wired` (requests `cpu`) still passes unchanged after this change — rejection still fires for a non-CUDA device request. Added `cuda_graph_decode_strategy_is_rejected_without_paged_attention` (`#[cfg(feature = "candle-cuda")]`) asserting the new, more specific "requires hardware_strategy.paged_attention" message fires on a CUDA request without `paged_attention: true`.
- [ ] 3.2 Add a CUDA-gated test loading a real Llama checkpoint with both `paged_attention: true` and `cuda_graph_decode: true`, asserting `generate(...)` runs a real captured/replayed decode and returns non-empty output matching (or closely matching) the non-captured paged-attention path's output for the same greedy prompt. **Blocked on 2.2/2.3.**
- [ ] 3.3 Regression: `cargo test -p core-host --features ai-inference ai_inference::` (0 regressions), `cargo clippy --workspace --all-targets --features core-host/ai-inference -- -D warnings -D clippy::unwrap_used`, `RUSTFLAGS="-D dead_code" cargo check -p core-host --features ai-inference`, `cargo fmt --all -- --check`. Passing today for the Section 2.1 gate-only change (part of the same regression run as `wire-flashinfer-decode-attention`'s 3.4); full re-verification needed once 2.2/2.3 land.

## 4. CI

- [ ] 4.1 Added a `cuda-quality` step exercising the Section 2.1 gate (`cuda_graph_decode_strategy_is_rejected_without_paged_attention`) — not yet confirmed green on real hardware. The real capture/replay generation step (Task 3.2) is still **blocked on 2.2/2.3**.

## 5. Docs

- [x] 5.1 Updated `docs/ai-inference-candle-llm-runtime.md`: new "CUDA Graph Decode Status" section (split out from the old combined section) describing the `paged_attention` gate dependency and the newly-discovered `write_new_kv` blocker.
- [x] 5.2 `CHANGELOG.md` entry (combined with `wire-flashinfer-decode-attention`'s, since both landed from the same tag bump) noting `cuda_graph_decode`'s gate now requires `paged_attention` but remains rejected pending the new fork-side fix.
- [ ] 5.3 Update issue #312: flashinfer_attention done, continuous batching's correctness fix landed, `cuda_graph_decode` blocked on a new (fourth) fork seam beyond the one originally scoped — not ready to close the issue.
