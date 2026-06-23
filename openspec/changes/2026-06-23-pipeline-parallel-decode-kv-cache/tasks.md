# Implementation Tasks

- [x] **Task 1: `PipelineStage` owns a persistent, decode-capable cache**
  - Implemented with a deviation from the literal task wording, recorded in `design.md` §5: the cache is **not** stored as a mutable field on `PipelineStage`. Instead, `PipelineStage::new_cache(&self) -> CandleResult<TensorParallelCache>` builds a fresh, decode-capable (`use_kv_cache: true`) cache, and a new `PipelineStage::forward_with_cache(&self, index_pos, input, cache: &mut TensorParallelCache)` takes the cache as an external `&mut` parameter — mirroring `tensor_parallel_llama.rs`'s already-established pattern (`TensorParallelLlama::forward(&self, ..., cache: &mut TensorParallelCache)`), so a cache built once per generation request is never shared across concurrent requests against the same `Arc<LoadedModel>`.
  - `PipelineStage::load` gained a `dtype: DType` parameter (stored as a new field) so `new_cache` can build a matching cache without depending on the dtype of whatever activation happens to flow through `forward_with_cache` first.
  - `PipelineStageExecutor::run_stage`'s signature is **unchanged** (`&self`, `layer_range`, no `index_pos`) — see Task 2.

- [x] **Task 2: Update `PipelineStageExecutor` and its implementors**
  - Deviation from the literal task wording (also recorded in `design.md` §5): the trait itself was **not** changed to `&mut self`/`index_pos`. `run_stage` now delegates to `forward_with_cache(0, input, &mut self.new_cache()?)` — a fresh, throwaway cache per call, preserving its exact existing prefill-only behavior and signature byte-for-byte. This keeps `run_pipeline`/`run_pipeline_microbatched` (and the existing TCP-transport test that calls `run_stage` directly) completely untouched, since those schedulers remain explicitly out of scope (no real wall-clock stage overlap, per the proposal's Non-Goals) and gain no benefit from decode support.
  - The actual decode entry point is the new `PipelineParallelLlama::forward_at` (Task 3), which calls `PipelineStage::forward_with_cache` directly rather than going through the `PipelineStageExecutor` trait at all.

- [x] **Task 3: `PipelineParallelLlama` decode entry point**
  - Added `PipelineParallelLlama::new_caches(&self) -> CandleResult<Vec<TensorParallelCache>>` (one fresh cache per stage, built per generation request) and `PipelineParallelLlama::forward_at(&self, index_pos, tokens, transports, caches: &mut [TensorParallelCache]) -> CandleResult<Tensor>`, which loops over stages calling `forward_with_cache` and forwarding the activation through `transports[i]`, exactly mirroring `TensorParallelLlama::forward`'s existing prefill/decode contract.
  - The existing `forward` method is unchanged and still used by the prefill-equivalence tests; `forward_at(0, ...)` with fresh caches is numerically identical to it for a single call.

- [x] **Task 4: Resolve the `StageTransport` connection-lifetime question**
  - Resolved as a non-issue for this change: `candle_llm_runtime.rs`'s production decode wiring (Task 5) always constructs `InProcessTransport` (via the new `pipeline_stage_transports` helper, promoted from the existing test-only prefill helper) — the same same-process, multi-device composition the prefill path already used. `InProcessTransport::send` has no connection state, so there is no per-token reconnect cost to resolve. Real cross-node decode over `TcpStageTransport` remains exercised only by the existing test (`pipeline_parallel_llama_hands_off_activations_over_a_real_tcp_socket`), unchanged from before this work, and is tracked as future work alongside genuine multi-process pipeline deployment (`design.md` §4 Out of scope).

- [x] **Task 5: Wire the decode loop into `candle_llm_runtime.rs`**
  - Replaced the `ParallelModel::Pipeline { .. } => Err(...)` arm with a real decode loop: builds `model.new_caches()` and `pipeline_stage_transports(model)` once per request, then drives `self.decode_loop(...)` with a closure calling `model.forward_at(index_pos, input, &transports, &mut caches)` — the same `decode_loop` driver already shared by the dense and tensor-parallel paths.
  - `ParallelModel::Pipeline` gained `config: Config`, `eos_tokens: Vec<u32>`, and `devices: Vec<Device>` fields (mirroring `ParallelModel::Tensor`), populated at load time from values already computed earlier in `try_load_with_topology`.
  - The existing `ParallelModel::Tensor` arm and the dense (`LoadedModel::Safetensors`/`Gguf`) paths are untouched; confirmed by running the full `ai_inference::` test suite (Task 6).

- [x] **Task 6: Tests**
  - Added `pipeline_parallel_llama_decodes_a_second_token_with_kv_cache` in `pipeline_parallel_llama.rs`, mirroring `tensor_parallel_llama_decodes_a_second_token_with_kv_cache`: a 2-stage `PipelineParallelLlama` runs a prefill (`forward_at(0, ...)`) then one decode step (`forward_at(3, ...)`) and the logits match a dense `Llama` reference run the same way, within `1e-3`.
  - No `run_pipeline`/`run_pipeline_microbatched` signature change was made (Task 2 deviation), so no regression test was needed there; their existing tests pass unmodified.
  - No `TcpStageTransport` connection-lifetime change was made (Task 4 resolution), so no new TCP decode test was needed; the existing single-call TCP test continues to pass.
  - Updated `pipeline_parallel_strategy_matches_dense_prefill_and_refuses_decode` (renamed to `pipeline_parallel_strategy_matches_dense_prefill_and_decodes`) in `candle_llm_runtime.rs` to assert full generation now succeeds, replacing its old assertion that generation was refused with a typed error.
  - Ran the full `core-host` `ai_inference::` suite (`cargo test -p core-host --features ai-inference`): 97 passed, 0 failed — no regressions in `tensor_parallel_llama::`/`parallel::`/`candle_llm_runtime::` tests.

- [x] **Task 7: Docs**
  - Updated the module-level doc comment at the top of `pipeline_parallel_llama.rs` to describe the decode-capable path (`forward_with_cache`/`forward_at`/`new_caches`) and the external-cache-per-request pattern, replacing the stale "prefill-only" framing.
  - Updated `ParallelModel`'s doc comment in `candle_llm_runtime.rs` to state both tensor and pipeline parallelism now support full decode.
  - Updated this change's own `specs/ai-inference/spec.md` delta's "Implementation status" section (see that file) and `design.md` (see its new §5) to record the cache-external correction.
