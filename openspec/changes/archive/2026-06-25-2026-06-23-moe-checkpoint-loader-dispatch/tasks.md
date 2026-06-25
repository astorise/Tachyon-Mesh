# Implementation Tasks

- [x] **Task 1: MoE config parsing**
  - Add a `MixtralConfigJson` struct (or equivalent) in `core-host/src/ai_inference/candle_llm_runtime.rs` capturing the HF Mixtral `config.json` fields needed: `hidden_size`, `intermediate_size`, `num_hidden_layers`, `num_attention_heads`, `num_key_value_heads`, `vocab_size`, `rms_norm_eps`, `num_local_experts`, `num_experts_per_tok`, plus whatever rotary/position fields the existing `LlamaConfig` parse already needs and Mixtral shares.
  - Branch on `ModelTypeProbe.model_type` *before* calling `load_llama_config` (`try_load_with_topology`, line ~596): `"llama"` keeps today's path completely unchanged; `"mixtral"` takes the new MoE parse path. Do not modify `load_llama_config` itself.
  - Reject `num_experts_per_tok != 1` at parse time with a typed `CandleLlmError::UnsupportedModel` (top-1-only scope, per `proposal.md` Non-Goals).

- [x] **Task 2: Per-layer dense/MoE detection and mixed block loading**
  - For each of `num_hidden_layers`, call the existing `detect_expert_count(tensor_names, layer_idx)` (`parallel.rs:895`) against the checkpoint's safetensors tensor names (obtainable via the existing safetensors-header-reading machinery already used elsewhere in this file for weight loading).
  - For a layer returning `Some(n)`, build that layer's MLP as `ExpertParallelMlp::load(vb, hidden_size, intermediate_size, n, &plan, &devices)`. For a layer returning `None`, load the existing dense `TensorParallelMlp` path unchanged.
  - Reuse `TensorParallelBlock`'s existing attention/norm sub-components for both layer kinds — only the MLP step differs.

- [x] **Task 3: `ExpertPlacementPlan` from `hardware_strategy.expert_device_map`**
  - Add `ExpertPlacementPlan::from_explicit_map_or_round_robin(expert_device_map, expert_count, device_count)` to `core-host/src/ai_inference/parallel.rs`, falling back to `round_robin` for any expert id the deployment's map omits.
  - Do not modify `round_robin` or `device_index_for`'s existing signatures/behavior.

- [x] **Task 4: `ExpertParallelLlama` model wrapper**
  - Add a new module (e.g. `core-host/src/ai_inference/expert_parallel_llama.rs`, mirroring the existing `tensor_parallel_llama.rs`/`pipeline_parallel_llama.rs` split) with an `ExpertParallelLlama` struct: embedding, a `Vec` of per-layer blocks (dense or MoE, per Task 2), final norm, LM head, and an owned `TensorParallelCache`.
  - Implement `load(weight_paths, dtype, moe_config, expert_device_map, devices) -> CandleResult<Self>` and `forward(&mut self, index_pos: usize, input: &Tensor) -> CandleResult<Tensor>`, the latter signature matching `TensorParallelLlama::forward`'s existing prefill/decode contract so the runtime dispatch can drive it identically.

- [x] **Task 5: Wire the load path in `candle_llm_runtime.rs`**
  - Replace the `GpuDistribution::ExpertParallelism => Err(CandleLlmError::UnsupportedModel { detail: "expert-parallel execution requires an MoE checkpoint loader, which is not yet implemented..." })` arm with a real load calling `ExpertParallelLlama::load`.
  - Add a `ParallelModel::Expert { model: Box<ExpertParallelLlama>, .. }` variant (mirroring the existing `ParallelModel::Tensor`/`Pipeline` variants) and a generation dispatch arm following the same prefill + per-token decode loop structure already used for `ParallelModel::Tensor`.
  - Confirm the existing dense, tensor-parallel, and pipeline-parallel paths are byte-for-byte unaffected (the `"llama"` branch of Task 1's config-parse split is untouched).

- [x] **Task 6: Tests**
  - Add a uniform-MoE fixture checkpoint (all layers declare expert tensors) and assert `ExpertParallelLlama`'s output matches a reference that runs each token through its selected expert individually (mirroring `expert_parallel_mlp_matches_per_token_dense_dispatch_reference`'s existing methodology, extended to a full model forward rather than just the MLP).
  - Add a mixed dense/MoE fixture checkpoint (some layers dense, some MoE) and assert correct per-layer dispatch (dense layers use `TensorParallelMlp`, MoE layers use `ExpertParallelMlp`, verified by output correctness, not just by absence of panics).
  - Add a decode test (prefill + at least one decode step) proving the KV cache is correctly shared across both dense and MoE layers in the same forward pass.
  - Add a test for `ExpertPlacementPlan::from_explicit_map_or_round_robin` (explicit overrides take precedence, omitted experts fall back to round-robin).
  - Add a test asserting `num_experts_per_tok != 1` is rejected at config-parse time with a typed error, not a panic or silent truncation.
  - Run the full `core-host` `ai_inference::` suite and confirm no regressions in the dense, tensor-parallel, or pipeline-parallel paths.

- [x] **Task 7: Docs**
  - Update `parallel.rs`'s `ExpertParallelMlp` doc comment (currently: "today nothing in this codebase loads an MoE checkpoint, so the dense path ... remains the only one ever exercised at runtime") to reflect that a real loader now exists.
  - Update this change's own `specs/ai-inference/spec.md` delta's "Implementation status" section once the work lands, naming what's real (top-1 routing, mixed dense/MoE layers) vs. still deferred (top-k > 1, combining with tensor/pipeline parallelism).
