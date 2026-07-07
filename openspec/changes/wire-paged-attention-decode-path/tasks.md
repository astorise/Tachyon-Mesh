## 0. Blocker: single-device Llama had no CUDA path at all — tracked by a separate change

Discovered while starting Section 3: `load_safetensors`, `load_gguf`, the
NVFP4 loader, and the LoRA-adapter loader all hardcode `Device::Cpu`
unconditionally, and `try_load_with_topology` rejects any
`distribution_mode: single` request for a non-`cpu` device before the
`paged_attention` check is reached. Only the tensor/pipeline/expert-parallel
engines reach real CUDA, and they use their own duplicated cache types, not
`candle_transformers::models::llama::Cache` (the type the new `set_paged_kv`
seam was added to). See `design.md`'s "Newly discovered blocker" section.

- [x] 0.1 Landed: [astorise/Tachyon-Mesh#341](https://github.com/astorise/Tachyon-Mesh/pull/341) merged to `main` 2026-07-07. Section 3+ (attaching `cache.set_paged_kv`) can now build on a real CUDA device for single-device Llama.

## 1. External prerequisite (astorise/candle fork, separate repository)

- [x] 1.1 Open a PR against `astorise/candle` adding the additive paged-KV seam to `candle-transformers::models::llama` proposed in `design.md`. Delivered shape keeps `Cache` a struct with `set_paged_kv`/`clear_paged_kv`/`paged_kv` per-layer slots (see design.md Decision 2 for why this differs slightly from the originally-proposed `Cache::Paged(..)` enum) — `CausalSelfAttention::forward` branches on `cache.paged_kv(block_idx)` and calls `flash_attn_varlen_paged_windowed` when set, falling back to today's contiguous path byte-for-byte otherwise. Tracked upstream as [astorise/candle#8](https://github.com/astorise/candle/issues/8) (closed).
- [x] 1.2 Tagged as `tachyon-v0.11.0-3`.
- [x] 1.3 Recorded. Sections 2-6 are now unblocked.

## 2. Block allocator and block table (core-host, CPU-testable, unblocked today)

Opened as [astorise/Tachyon-Mesh#342](https://github.com/astorise/Tachyon-Mesh/pull/342), stacked on [#341](https://github.com/astorise/Tachyon-Mesh/pull/341) (`enable-single-device-llama-cuda-execution`) since this module's tasks.md edit lands in the same directory #341 introduces.

- [x] 2.1 Added `core-host/src/ai_inference/paged_kv.rs` with `PagedBlockPool` (fixed `page_block_size`, free-list `allocate_block`/`free_blocks`) and `SequenceBlockTable` (per-sequence `Vec<BlockId>`, `grow_to`/`free`). Module-wide `#![allow(dead_code)]` since nothing calls it yet (Section 3 is the caller) — matches the precedent in `parallel.rs` for shipped-ahead-of-its-wiring code, and keeps the `-D dead_code` feature-matrix gate green in the meantime.
- [ ] 2.2 Partially done — implemented the generic sizing primitive — `PagedBlockPool::try_new_within_budget(page_block_size, bytes_per_block, budget_bytes, min_blocks)` fits as many blocks as a byte budget allows and rejects with a typed `PagedKvError::BudgetTooSmall` if it can't fit `min_blocks`. **Not done**: reading an actual `hardware_strategy` config knob, sourcing the real budget from NVML free-VRAM telemetry (`discover_cluster_topology()`), and turning `PagedKvError` into a `CandleLlmError` that fails the load — all of that needs `candle_llm_runtime.rs`'s load path, i.e. Section 3.
- [x] 2.3 Added `build_block_table_tensor` (`(batch_size, max_blocks)`, zero-padded) and `build_cumulative_seqlens_tensor` (`(batch_size + 1,)`), matching `flash_attn_varlen_paged_windowed`/`PagedKvCache`'s expected layout exactly (field doc comments cite the shapes).
- [x] 2.4 11 unit tests in `paged_kv.rs`: allocation/free through the pool, exhaustion as a typed error, budget-too-small rejection, budget-fits-as-many-as-possible sizing, block-table growth (whole-block, idempotent, exhaustion), freeing returns blocks, block-table tensor padding (including the zero-sequence edge case), cumulative-seqlens tensor (including the zero-sequence edge case). `cargo test -p core-host --features ai-inference ai_inference::` → 181/181 passed (170 + 11 new), 0 regressions. `cargo clippy --workspace --all-targets --features core-host/ai-inference -- -D warnings -D clippy::unwrap_used` and `RUSTFLAGS="-D dead_code" cargo check -p core-host --features ai-inference` both clean. `cargo fmt --all -- --check` clean.

## 3. Runtime wiring (unblocked, code complete 2026-07-07)

**Discovered starting this section**: `candle-flash-attn`'s kernels (so
`flash_attn_varlen_paged_windowed`) hard-error on F32 —
`"flash-attn is only supported for f16/bf16"` — but the whole single-device
Llama path (`VarBuilder`, `Cache`) loads/computes in F32. Standard practice
industry-wide (vLLM, TensorRT-LLM also require FP16/BF16 for fused attention
kernels), but it means paged attention needed a dtype switch, not just a
block table, added to this section's scope (user decision: include it here
rather than as a fourth separate prerequisite change).

- [x] 3.1 Bump `core-host/Cargo.toml`'s pinned `astorise/candle` tag to `tachyon-v0.11.0-3` and add `candle-transformers/flash-attn` to the `candle-cuda` feature (the paged path is stubbed to `unimplemented!()` in candle-transformers without that feature). Landed in #341.
- [x] 3.2 `try_load_with_topology`'s `paged_attention` rejection is now gated the same way as the single-device CUDA gate: `paged_attention_supported = single_device_cuda_supported && requested_device != "cpu"`; every other architecture/build/device combination still fails closed with the existing typed error (reworded to name the Llama+CUDA restriction). `load_safetensors` selects `DType::BF16` (instead of `F32`) for the `VarBuilder`/`Cache` only when `architecture == Llama && strategy.paged_attention`; every other case is byte-for-byte unchanged. A new `build_paged_attention_runtime` (in `candle_llm_runtime.rs`) sizes a `PagedBlockPool` from real NVML free-VRAM telemetry (`discover_cluster_topology()`, queried *after* weights are loaded so the budget reflects what's actually left; fixed heuristic: 50% of remaining free VRAM, `min_blocks` = one full-length sequence at `max_position_embeddings`) and allocates one `(key_cache, value_cache)` tensor pair per transformer layer, then attaches `cache.set_paged_kv(layer_idx, ..)` for every layer on every forward step via a new `paged_llama_forward` helper.
- [x] 3.3 `SingleDeviceBackend::Llama` gained a `paged: Option<PagedAttentionRuntime>` field (model-level, built once at load time, reused across requests) and `SingleDeviceBackend::decode`'s Llama arm branches on it: `Some` runs a fresh per-request `PagedSequenceGuard`-owned `SequenceBlockTable`, grown one block at a time by `paged_llama_forward` on every prefill/decode step, freed back to the shared pool on drop (including on early return/error, via `Drop`); `None` is the existing contiguous/prefix-cache path, byte-for-byte unchanged.
- [x] 3.4 Prefix caching: paged requests never call `llama_prefill_with_prefix_cache` (they go through `decode_loop` directly with the paged forward closure), so there's no cross-contamination — a clean non-interaction, not a rejection. LoRA adapters: `generate_with_adapter_streaming` already reloads a fully independent CPU/F32 `Llama` instance per request regardless of the main backend's state, so it was already unaffected — verified, not changed. Speculative decoding: `greedy_next_token_id`/`last_logits` build a **fresh contiguous `Cache`** from the (possibly BF16) shared model, which would dtype-mismatch without paged KV state attached — `generate_speculative_streaming` now falls back to plain `generate_streaming` when either the target or the draft has `is_paged_attention_enabled()` (a new helper), the same pattern that function already uses for an incompatible tokenizer.

## 4. Tests

- [x] 4.1 `paged_attention_strategy_is_rejected_until_block_tables_are_wired` (cpu-request case) still passes unchanged. Added `paged_attention_strategy_is_rejected_for_a_non_llama_architecture` (Qwen2 fixture, `cuda` request, always-on regardless of `candle-cuda`) covering the architecture-gate case Task 3.2 added.
- [x] 4.2 Added `#[cfg(feature = "candle-cuda")]` test `single_device_llama_paged_attention_generates_a_real_decode_on_cuda`: loads the `tiny` fixture with `paged_attention: true` on `cuda`, asserts a real non-empty decode, and asserts a **second** request against the same (shared-pool) runtime produces identical greedy output — proving the block pool is correctly reused/freed across requests, not just that one request works. Not runnable on this dev machine (GPU present, but local `nvcc`/MSVC toolchain mismatch — see `enable-single-device-llama-cuda-execution`'s design.md); needs a real `cuda-quality` run to be considered proven, not just compiled.
- [x] 4.3 `cargo test -p core-host --features ai-inference ai_inference::` → 182/182 passed (181 + 1 new always-on test; the CUDA-gated test isn't compiled in this run), 0 regressions. `cargo clippy --workspace --all-targets --features core-host/ai-inference -- -D warnings -D clippy::unwrap_used`, `RUSTFLAGS="-D dead_code" cargo check -p core-host --features ai-inference`, and `cargo fmt --all -- --check` all clean. `cargo check -p core-host --features ai-inference` (production code, not cfg-gated behind `candle-cuda`) already type-checks the full paged-attention implementation on this CPU-only sandbox — only the two `#[cfg(feature = "candle-cuda")]` test functions and actual runtime correctness remain unverified locally.

## 5. CI and bench proof

- [x] 5.1 Added a `cuda-quality` step ("Run paged-attention CUDA execution proof") running Task 4.2's test on `arc-gpu-runners`, right after the single-device CUDA execution proof step. YAML validated with a `yaml.safe_load` parse; not yet exercised by an actual CI run.
- [ ] 5.2 Not done — a before/after decode-throughput bench entry for paged vs. contiguous KV cache (issue #308's bench harness) is a follow-up.

## 6. Docs

- [x] 6.1 Rewrote `docs/ai-inference-candle-llm-runtime.md`'s "PagedAttention Status" section: Llama+CUDA-only enablement, the BF16 dtype switch and why, the pool-sizing heuristic, and the speculative-decoding/continuous-batching/LoRA interaction notes.
- [x] 6.2 `CHANGELOG.md` entry added under `## Unreleased`.
- [x] 6.3 Noted in this file's own header (Section 3) and the CHANGELOG entry that `cuda_graph_decode`, `flashinfer_attention`, and continuous batching remain open per issue #312; a configurable pool-sizing knob (no `hardware_strategy` field yet, fixed 50%-of-free-VRAM heuristic) and broadening beyond Llama are noted as open follow-ups in the docs update (6.1) and this section's own task descriptions (2.2, 3.2).
