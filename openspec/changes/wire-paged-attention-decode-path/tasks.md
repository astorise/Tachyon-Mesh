## 0. Blocker: single-device Llama had no CUDA path at all — tracked by a separate change

Discovered while starting Section 3: `load_safetensors`, `load_gguf`, the
NVFP4 loader, and the LoRA-adapter loader all hardcode `Device::Cpu`
unconditionally, and `try_load_with_topology` rejects any
`distribution_mode: single` request for a non-`cpu` device before the
`paged_attention` check is reached. Only the tensor/pipeline/expert-parallel
engines reach real CUDA, and they use their own duplicated cache types, not
`candle_transformers::models::llama::Cache` (the type the new `set_paged_kv`
seam was added to). See `design.md`'s "Newly discovered blocker" section.

- [ ] 0.1 Land `openspec/changes/enable-single-device-llama-cuda-execution/` (a separate, narrower change scoped to just the CUDA device baseline for single-device Llama, no paged attention) on `main`. Do not attach `cache.set_paged_kv` to a load path that can never construct a CUDA device — Sections 3+ below are blocked until that change's Task 1 (device resolution) and Task 2 (generate-time device threading) are done.

## 1. External prerequisite (astorise/candle fork, separate repository)

- [x] 1.1 Open a PR against `astorise/candle` adding the additive paged-KV seam to `candle-transformers::models::llama` proposed in `design.md`. Delivered shape keeps `Cache` a struct with `set_paged_kv`/`clear_paged_kv`/`paged_kv` per-layer slots (see design.md Decision 2 for why this differs slightly from the originally-proposed `Cache::Paged(..)` enum) — `CausalSelfAttention::forward` branches on `cache.paged_kv(block_idx)` and calls `flash_attn_varlen_paged_windowed` when set, falling back to today's contiguous path byte-for-byte otherwise. Tracked upstream as [astorise/candle#8](https://github.com/astorise/candle/issues/8) (closed).
- [x] 1.2 Tagged as `tachyon-v0.11.0-3`.
- [x] 1.3 Recorded. Sections 2-6 are now unblocked.

## 2. Block allocator and block table (core-host, CPU-testable, unblocked today)

- [x] 2.1 Added `core-host/src/ai_inference/paged_kv.rs` with `PagedBlockPool` (fixed `page_block_size`, free-list `allocate_block`/`free_blocks`) and `SequenceBlockTable` (per-sequence `Vec<BlockId>`, `grow_to`/`free`). Module-wide `#![allow(dead_code)]` since nothing calls it yet (Section 3 is the caller) — matches the precedent in `parallel.rs` for shipped-ahead-of-its-wiring code, and keeps the `-D dead_code` feature-matrix gate green in the meantime.
- [ ] 2.2 Partially done — implemented the generic sizing primitive — `PagedBlockPool::try_new_within_budget(page_block_size, bytes_per_block, budget_bytes, min_blocks)` fits as many blocks as a byte budget allows and rejects with a typed `PagedKvError::BudgetTooSmall` if it can't fit `min_blocks`. **Not done**: reading an actual `hardware_strategy` config knob, sourcing the real budget from NVML free-VRAM telemetry (`discover_cluster_topology()`), and turning `PagedKvError` into a `CandleLlmError` that fails the load — all of that needs `candle_llm_runtime.rs`'s load path, i.e. Section 3.
- [x] 2.3 Added `build_block_table_tensor` (`(batch_size, max_blocks)`, zero-padded) and `build_cumulative_seqlens_tensor` (`(batch_size + 1,)`), matching `flash_attn_varlen_paged_windowed`/`PagedKvCache`'s expected layout exactly (field doc comments cite the shapes).
- [x] 2.4 11 unit tests in `paged_kv.rs`: allocation/free through the pool, exhaustion as a typed error, budget-too-small rejection, budget-fits-as-many-as-possible sizing, block-table growth (whole-block, idempotent, exhaustion), freeing returns blocks, block-table tensor padding (including the zero-sequence edge case), cumulative-seqlens tensor (including the zero-sequence edge case). `cargo test -p core-host --features ai-inference ai_inference::` → 181/181 passed (170 + 11 new), 0 regressions. `cargo clippy --workspace --all-targets --features core-host/ai-inference -- -D warnings -D clippy::unwrap_used` and `RUSTFLAGS="-D dead_code" cargo check -p core-host --features ai-inference` both clean. `cargo fmt --all -- --check` clean.

## 3. Runtime wiring (blocked on Section 0)

- [x] 3.1 Bump `core-host/Cargo.toml`'s pinned `astorise/candle` tag to `tachyon-v0.11.0-3` and add `candle-transformers/flash-attn` to the `candle-cuda` feature (the paged path is stubbed to `unimplemented!()` in candle-transformers without that feature).
- [ ] 3.2 In `try_load_with_topology` (`candle_llm_runtime.rs`), replace the unconditional `paged_attention` rejection with: build the block pool/table and call `cache.set_paged_kv(block_idx, ..)` per layer when the binding is Llama on a CUDA device; keep the existing typed rejection for every other architecture/device combination.
- [ ] 3.3 Wire prefill and decode call sites for Llama to grow each sequence's `SequenceBlockTable` by a block every `page_block_size` tokens and free its blocks on sequence completion/eviction.
- [ ] 3.4 Confirm LoRA adapters, speculative decoding, and prefix caching either compose with the paged cache or explicitly reject the combination with a typed error (do not silently ignore one feature when both are requested).

## 4. Tests

- [ ] 4.1 Update `paged_attention_strategy_is_rejected_until_block_tables_are_wired` to assert the rejection still fires for non-Llama architectures and non-CUDA devices after this change.
- [ ] 4.2 Add a CUDA-gated (`candle-cuda` feature) test that loads a small real Llama checkpoint with `paged_attention: true` and asserts `generate(...)` runs a real decode and returns non-empty, non-mocked output, matching the non-paged path's output for a greedy prompt. This dev machine has a GPU (RTX 3070) but `--features candle-cuda` does not currently compile here (`nvcc`/MSVC toolset mismatch, see design.md) — only `cuda-quality` (`arc-gpu-runners`) can validate this task until that local toolchain gap is fixed separately.
- [ ] 4.3 Regression: `cargo test -p core-host --features ai-inference ai_inference::` (0 regressions) and `cargo clippy --workspace --all-targets --features core-host/ai-inference -- -D warnings -D clippy::unwrap_used`.

## 5. CI and bench proof

- [ ] 5.1 Add a `cuda-quality` step exercising Task 4.2's paged-attention generation on `arc-gpu-runners`, modeled on the existing NCCL/NVML proof job.
- [ ] 5.2 Add a before/after decode-throughput bench entry for paged vs. contiguous KV cache (issue #308's bench harness).

## 6. Docs

- [ ] 6.1 Update `docs/ai-inference-candle-llm-runtime.md` describing the paged-attention path, its Llama+CUDA-only scope, and the block-pool sizing knob.
- [ ] 6.2 `CHANGELOG.md` entry noting `hardware_strategy.paged_attention` is now enabled for Llama/CUDA (still rejected elsewhere), and that this depended on an `astorise/candle` tag bump.
- [ ] 6.3 Note in the change (or a follow-up issue) that `cuda_graph_decode`, `flashinfer_attention`, and continuous batching remain open per issue #312 and are tracked as separate changes once this one is on `main`.
