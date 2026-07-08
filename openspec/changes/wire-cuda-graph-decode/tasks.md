## 1. External prerequisite (astorise/candle fork, separate repository)

- [ ] 1.1 Land an additive `Cache::set_decode_position`-style seam in `candle-transformers::models::llama` (proposed shape in `design.md` Decision 1): a persistent per-layer device tensor the rotary-embedding application reads via `index_select`/gather instead of `narrow(0, index_pos, seq_len)` whenever attached, decode-step only. Tracked upstream as [astorise/candle#12](https://github.com/astorise/candle/issues/12).
- [ ] 1.2 Tag a new fork release once that PR merges and its own tests pass.
- [ ] 1.3 Bump `core-host/Cargo.toml`'s pinned `astorise/candle` tag to the new release. Sections 2+ below are blocked until this lands.

## 2. Runtime wiring (blocked on Section 1)

- [ ] 2.1 In `try_load_with_topology`, replace the unconditional `cuda_graph_decode` rejection with a gate requiring Llama + CUDA + `paged_attention: true` (design.md Decision 2); reject with a typed error naming the `paged_attention` dependency when it's missing, and keep the existing rejection for every other combination.
- [ ] 2.2 Change `build_paged_attention_runtime`'s per-request block-table/seqlens construction (or add a captured-mode variant) to pre-allocate `(1, min_blocks)`/`(2,)` tensors once per request and write into them in place (`slice_set`/`scatter_set`) as the sequence grows, instead of `build_block_table_tensor`/`build_cumulative_seqlens_tensor` allocating a fresh tensor sized to the currently-used block count on every call (design.md Decision 3). Keep the existing (non-captured) behavior for `paged_attention` without `cuda_graph_decode`.
- [ ] 2.3 Implement the capture/replay orchestration in `paged_llama_forward` (or a sibling): warm-up call, `CudaGraph::capture` on the first decode step, `graph.replay()` for subsequent steps, scoped to one request/`PagedSequenceGuard` lifetime (design.md Decision 4).
- [ ] 2.4 Confirm LoRA adapters, speculative decoding, and prefix caching either compose or are explicitly rejected for a `cuda_graph_decode` deployment, following the same audit pattern as `wire-paged-attention-decode-path` Task 3.4.

## 3. Tests

- [ ] 3.1 Update `cuda_graph_decode_strategy_is_rejected_until_gpu_decode_is_wired` (or equivalent) to assert the rejection still fires for non-Llama architectures, non-CUDA devices/builds, and `cuda_graph_decode` without `paged_attention`, after this change.
- [ ] 3.2 Add a CUDA-gated test loading a real Llama checkpoint with both `paged_attention: true` and `cuda_graph_decode: true`, asserting `generate(...)` runs a real captured/replayed decode and returns non-empty output matching (or closely matching) the non-captured paged-attention path's output for the same greedy prompt.
- [ ] 3.3 Regression: `cargo test -p core-host --features ai-inference ai_inference::` (0 regressions), `cargo clippy --workspace --all-targets --features core-host/ai-inference -- -D warnings -D clippy::unwrap_used`, `RUSTFLAGS="-D dead_code" cargo check -p core-host --features ai-inference`, `cargo fmt --all -- --check`.

## 4. CI

- [ ] 4.1 Add a `cuda-quality` step exercising Task 3.2's cuda-graph-decode generation on `arc-gpu-runners`.

## 5. Docs

- [ ] 5.1 Update `docs/ai-inference-candle-llm-runtime.md`'s "CUDA Graphs and FlashInfer Status" section to describe the now-enabled CUDA Graph path (Llama+CUDA+paged_attention only, no recapture within a request).
- [ ] 5.2 `CHANGELOG.md` entry noting `hardware_strategy.cuda_graph_decode` is now enabled for paged-attention Llama/CUDA deployments (still rejected elsewhere, including without `paged_attention`), and that this depended on an `astorise/candle` tag bump.
- [ ] 5.3 Update issue #312: with this, all four items are done — close the issue (or hand off to whoever owns that decision at the time).
