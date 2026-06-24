# ai-inference Delta

## MODIFIED Requirements

### Requirement: The runtime MUST execute expert-parallel inference for Mixture-of-Experts checkpoints
For checkpoints declaring expert tensors (e.g. Mixtral-style `model_type: mixtral` checkpoints), the runtime SHALL load the checkpoint, partition experts across the configured GPU/node set, and SHALL route each token only to the device(s) hosting its selected expert, rather than rejecting expert-parallel deployments outright or replicating all experts on every device.

#### Scenario: An MoE checkpoint is loaded and partitioned across devices
- **GIVEN** a model deployment configured with `distribution_mode: expert_parallelism` and a checkpoint whose `config.json` declares `model_type: mixtral`
- **WHEN** the model broker loads the model
- **THEN** the runtime parses the checkpoint's MoE-specific config fields (`num_local_experts`, `num_experts_per_tok`)
- **AND** partitions experts across the configured device set per the deployment's `hardware_strategy.expert_device_map`, falling back to an even round-robin placement for any expert the map does not explicitly pin
- **AND** does not load a full replica of every expert onto every device

#### Scenario: Mixed dense and MoE layers in the same checkpoint load correctly
- **GIVEN** a checkpoint where some transformer layers declare expert tensors and others do not
- **WHEN** the model broker loads the model
- **THEN** layers without expert tensors execute the existing dense MLP path unchanged
- **AND** layers with expert tensors execute the expert-parallel routed path
- **AND** both layer kinds share the same attention and KV-cache machinery within one forward pass

#### Scenario: Tokens are routed only to their selected expert's device
- **WHEN** the gate layer selects the top-1 expert for a token
- **THEN** the runtime forwards that token's hidden state only to the device hosting the selected expert
- **AND** non-MoE checkpoints continue to execute the existing dense path unchanged

#### Scenario: An MoE deployment generates more than one token
- **GIVEN** a successfully loaded expert-parallel deployment
- **WHEN** a generation request is submitted with `max_tokens > 1`
- **THEN** the runtime completes an initial prefill pass followed by per-token decode steps
- **AND** the KV cache persists correctly across decode steps for both dense and MoE layers

#### Scenario: Top-k greater than one is rejected at load time
- **GIVEN** a checkpoint whose config declares `num_experts_per_tok > 1`
- **WHEN** the model broker attempts to load it under `distribution_mode: expert_parallelism`
- **THEN** the runtime rejects the deployment with a typed `UnsupportedModel` error
- **AND** does not silently truncate routing to top-1

## Implementation status as of this change

`detect_expert_count`, `ExpertPlacementPlan`, `ExpertMlp`, and `ExpertParallelMlp`
(`core-host/src/ai_inference/parallel.rs`) are pre-existing, numerically verified
primitives unchanged by this work. This change adds the previously-missing caller:
MoE config parsing (`load_mixtral_config`, a `RawMixtralConfig`/`MixtralConfigJson`
pair) branching ahead of the Llama-only `load_llama_config`, per-layer dense/MoE
detection via a new `ExpertParallelLlama` model wrapper
(`core-host/src/ai_inference/expert_parallel_llama.rs`), and a real
`GpuDistribution::ExpertParallelism` dispatch arm in `candle_llm_runtime.rs`,
replacing the previous unconditional `CandleLlmError::UnsupportedModel` rejection.
`ExpertParallelLlama::forward` takes its `TensorParallelCache` as an external
`&mut` parameter rather than as an owned field (the same revision
`pipeline-parallel-decode-kv-cache` made to its own design for the same reason:
the loaded model is shared behind `Arc<LoadedModel>` across concurrent requests,
so a cache owned by the model would let concurrent requests corrupt each other's
KV state). `ParallelModel::Expert` reuses the same `decode_loop` driver and
single-cache decode pattern as `ParallelModel::Tensor`, since expert-parallel
attention is dense and replicated identically. Verified by per-layer MoE/dense
dispatch tests, a dense-reference equivalence test (single-expert top-1 routing
is numerically identical to the dense path), a multi-token decode test, and the
full `ai_inference::` suite (104 tests, 0 regressions). Top-k routing greater
than one and combining expert-parallelism with tensor- or pipeline-parallelism
in the same deployment remain out of scope and are tracked as follow-ups.
