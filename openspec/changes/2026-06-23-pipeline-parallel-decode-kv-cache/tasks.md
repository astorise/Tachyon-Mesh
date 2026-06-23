# Implementation Tasks

- [ ] **Task 1: `PipelineStage` owns a persistent, decode-capable cache**
  - In `core-host/src/ai_inference/pipeline_parallel_llama.rs`, add a `cache: TensorParallelCache` field to `PipelineStage`, constructed once in `PipelineStage::load` with `TensorParallelCache::new(true, dtype, cfg, device)` (flipping the existing `use_kv_cache` argument from `false` to `true`).
  - Change `run_stage` to take `&mut self` and `index_pos: usize` instead of constructing a fresh, throwaway cache and hard-coding `index_pos = 0` on every call.
  - Drop the `layer_range` parameter from `run_stage` (it was already documented as a pure sanity-check assertion, never load-bearing) since the stage already owns its own range.

- [ ] **Task 2: Update `PipelineStageExecutor` and its implementors**
  - In `core-host/src/ai_inference/parallel.rs`, change the `PipelineStageExecutor` trait's `run_stage` signature to `fn run_stage(&mut self, index_pos: usize, input: &Tensor) -> CandleResult<Tensor>`.
  - Update `ClosureStageExecutor` (test-only) and any other implementor to match.
  - Update `run_pipeline` and `run_pipeline_microbatched` (and `PipelineDepthGate`'s call sites) to hold `&mut` references to stages and thread an `index_pos` through, defaulting existing prefill-only callers to `index_pos = 0` so today's tests keep passing unmodified in their prefill-only form.

- [ ] **Task 3: `PipelineParallelLlama` decode entry point**
  - Add `forward_prefill` (index_pos = 0) and `forward_decode(index_pos, ..)` methods to `PipelineParallelLlama`, both delegating to a shared `forward_at(index_pos, tokens, transports)` that loops `self.stages.iter_mut()` calling the now-`&mut self` `run_stage`.
  - Keep the existing `forward` method as a thin wrapper around `forward_prefill` (or rename call sites) so the existing prefill-equivalence tests (`pipeline_parallel_llama_matches_dense_reference_on_a_real_checkpoint`, `pipeline_parallel_llama_hands_off_activations_over_a_real_tcp_socket`) continue to pass with minimal changes.

- [ ] **Task 4: Resolve the `StageTransport` connection-lifetime question**
  - Determine whether `TcpStageTransport`'s current connect-per-`send` behavior is acceptable for a multi-step decode loop, or whether it needs to hold a persistent connection across the calls of one generation request (per `design.md` §3).
  - If a persistent connection is needed, implement it inside `TcpStageTransport` (and its `serve_one` peer-side counterpart, which may need to become a loop rather than a single serve-and-return) without changing the `StageTransport` trait's public `send` signature, unless investigation during implementation shows the trait itself needs a new method — document that decision here if so.

- [ ] **Task 5: Wire the decode loop into `candle_llm_runtime.rs`**
  - Replace the `ParallelModel::Pipeline { .. } => Err(self.execution_error("pipeline-parallel generation ... is not yet wired ..."))` arm with a real decode loop: call `forward_prefill` once, sample the first token, then repeatedly call `forward_decode(index_pos, ..)` with `index_pos` advancing each iteration, following the same sampling/stop-condition structure already used by the adjacent `ParallelModel::Tensor` dispatch arm.
  - Ensure the existing `ParallelModel::Tensor` arm and the dense (`LoadedModel::Safetensors`/`Gguf`) path are completely untouched by this change.

- [ ] **Task 6: Tests**
  - Add a decode-equivalence test in `pipeline_parallel_llama.rs` mirroring `tensor_parallel_llama_decodes_a_second_token_with_kv_cache`: run a 2-stage `PipelineParallelLlama` through a prefill + one decode step and assert the logits match a dense `Llama` reference run the same way, within `1e-3`.
  - Add/update a test exercising `run_pipeline`/`run_pipeline_microbatched` with the new `&mut self`/`index_pos` signature to confirm no behavioral regression in the existing micro-batch scheduling tests.
  - If Task 4 changes `TcpStageTransport`'s connection lifetime, add a test proving a multi-step decode sequence over the real TCP transport produces the same result as the in-process transport across multiple decode steps (not just one prefill call, as today's existing TCP test covers).
  - Run the full `core-host` `ai_inference::` suite and confirm no regressions in `tensor_parallel_llama::`/`parallel::`/`candle_llm_runtime::` tests.

- [ ] **Task 7: Docs**
  - Update the module-level doc comment at the top of `pipeline_parallel_llama.rs` (lines 1-20), which currently states the engine is "prefill-only" — correct this once decode is wired, and note any remaining caveats (e.g. real stage overlap is still out of scope).
  - Update this change's own `specs/ai-inference/spec.md` delta's "Implementation status" section once the work lands, naming what's real vs. still deferred (e.g. real wall-clock multi-process pipeline overlap).
