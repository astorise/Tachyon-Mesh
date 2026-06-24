# ai-inference Delta

## MODIFIED Requirements

### Requirement: The runtime MUST execute pipeline-parallel inference across multiple nodes
When a model deployment is configured with `hardware_strategy.distribution_mode: pipeline_parallelism`, the runtime SHALL assign contiguous layer ranges to distinct nodes/GPUs, SHALL stream activations between pipeline stages over a point-to-point transport implementing `StageTransport`, and SHALL support full autoregressive generation (prefill followed by an arbitrary number of decode steps), not prefill alone.

#### Scenario: Layers are split across pipeline stages
- **GIVEN** a model deployment configured with `distribution_mode: pipeline_parallelism` across N nodes
- **WHEN** the model broker loads the model
- **THEN** each node is assigned a contiguous, non-overlapping range of layers
- **AND** each node executes its layer range with a real transformer-block forward pass

#### Scenario: A pipeline-parallel deployment generates more than one token
- **GIVEN** a model deployment configured with `distribution_mode: pipeline_parallelism` and successfully loaded
- **WHEN** a generation request is submitted with `max_tokens > 1`
- **THEN** the runtime completes an initial prefill pass across all stages
- **AND** completes a decode pass for each subsequent token, each stage reusing a persistent per-stage KV cache rather than rebuilding it from scratch
- **AND** the final output is numerically equivalent (within floating-point tolerance) to a dense single-device reference run of the same model and prompt for the same number of tokens

#### Scenario: Pipeline depth bounds in-flight micro-batches
- **GIVEN** a pipeline-parallel deployment with a configured pipeline depth
- **WHEN** multiple inference requests are in flight concurrently
- **THEN** the scheduler admits at most the configured number of micro-batches into the pipeline at once
- **AND** additional requests queue rather than unboundedly growing per-stage memory usage

## Implementation status as of this change

`PipelineStage` (`core-host/src/ai_inference/pipeline_parallel_llama.rs`) gained
`new_cache`/`forward_with_cache`, and `PipelineParallelLlama` gained `new_caches`/`forward_at`,
which build one decode-capable `TensorParallelCache` per stage fresh per generation request and
thread `index_pos` through every stage on every call — the cache is owned by the caller, not
stored as a mutable field on `PipelineStage`, so concurrent requests against the same loaded
model never share or corrupt each other's KV state (mirroring `tensor_parallel_llama.rs`'s
existing per-request cache pattern; see `design.md` §5 for why the originally-designed
cache-as-field/`&mut self` approach was revised during implementation).
`candle_llm_runtime.rs`'s dispatch for `ParallelModel::Pipeline` runs a real prefill-then-decode
loop through the shared `decode_loop` driver instead of returning the previous
`"pipeline-parallel generation ... is not yet wired"` error; `ParallelModel::Pipeline` gained
`config`/`eos_tokens`/`devices` fields to support this. `PipelineStageExecutor::run_stage`'s
trait signature is unchanged (still `&self`, prefill-only, fresh throwaway cache per call) and
remains the path used by the still-prefill-only `run_pipeline`/`run_pipeline_microbatched`
schedulers. Verified by a new decode-equivalence test
(`pipeline_parallel_llama_decodes_a_second_token_with_kv_cache`) and the full `ai_inference::`
suite (97 tests, 0 regressions). Real wall-clock, multi-process stage overlap (one OS
thread/process genuinely executing each stage concurrently, as opposed to the existing
in-process sequential `run_pipeline_microbatched` admission scheduler) and real cross-node
decode over `TcpStageTransport` (production wiring uses `InProcessTransport` only) remain out
of scope and are tracked as separate follow-ups.
