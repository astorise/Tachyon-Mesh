# ai-inference Delta

## ADDED Requirements

### Requirement: A model deployment's `hardware_strategy` MUST select the parallel execution engine at load time
When a model deployment declares `hardware_strategy.distribution_mode` other than `single`, the runtime SHALL carry that strategy from configuration into model loading and SHALL construct the corresponding parallel engine (tensor-, pipeline-, or expert-parallel) instead of the dense single-device path. A `single` (or absent) strategy SHALL load the existing single-device path with no behavioural change.

#### Scenario: A tensor-parallel deployment is dispatched to the tensor-parallel engine
- **GIVEN** a model binding whose `hardware_strategy.distribution_mode` is `tensor_parallelism` with two device IDs
- **WHEN** the runtime loads the model
- **THEN** the binding's strategy is threaded into `try_load`
- **AND** the model is loaded as a tensor-parallel engine across the configured devices
- **AND** generation produces output numerically equivalent (within floating-point tolerance) to the dense single-device path on the same checkpoint

#### Scenario: A pipeline-parallel deployment is dispatched to the pipeline engine
- **GIVEN** a model binding whose `distribution_mode` is `pipeline_parallelism` with contiguous `stage_layer_ranges`
- **WHEN** the runtime loads the model
- **THEN** the model is loaded as a pipeline-parallel engine with the configured stage ranges
- **AND** a prefill request returns prompt logits equivalent to the dense reference
- **AND** a token-streaming (decode) request returns a typed "decode not yet supported for pipeline parallelism" error rather than incorrect output

#### Scenario: An expert-parallel deployment is validated but refused until a MoE loader exists
- **GIVEN** a model binding whose `distribution_mode` is `expert_parallelism`
- **WHEN** the runtime loads the model
- **THEN** the plan is validated against the discovered hardware topology
- **AND** the load returns a typed error indicating that a full MoE checkpoint loader is not yet implemented (only the numerically-verified per-expert `ExpertParallelMlp` primitive exists), rather than constructing a non-existent full MoE model or silently downgrading to the dense path

#### Scenario: Single-device deployments are byte-for-byte unaffected
- **WHEN** a model binding declares `distribution_mode: single` or carries no `hardware_strategy`
- **THEN** the existing `Safetensors`/`Gguf` single-device load path executes unchanged
- **AND** no parallel dispatch, topology discovery, or strategy plumbing is invoked

### Requirement: The runtime MUST validate a parallel plan against discovered hardware before loading weights
Before constructing any parallel engine, the runtime SHALL validate the requested plan against the cluster's discovered hardware topology (device count, interconnect class, per-shard VRAM) and SHALL abort the load with a typed topology error — loading no weights — when the plan cannot be satisfied. This hardware-aware check is in addition to the structural plan validation already performed by the config API.

#### Scenario: A plan requesting more devices than exist is rejected before any load
- **GIVEN** a binding requesting a parallel plan across more devices than `discover_cluster_topology()` reports
- **WHEN** the runtime attempts to load the model
- **THEN** `try_load` fails with a typed topology error mapped from `TopologyError::InsufficientDeviceCount`
- **AND** no model weights are allocated

### Requirement: GPU execution MUST be served when the candle CUDA backend is compiled in, and refused with a typed error otherwise
The runtime SHALL accept a GPU `device` request only on a build where the candle CUDA backend is compiled in. On a build without the CUDA backend, a GPU request SHALL continue to return the existing typed unsupported-execution error, and parallel engines SHALL run on CPU device stand-ins.

#### Scenario: GPU request on a CUDA-less build is refused unchanged
- **GIVEN** a build without the `candle-cuda` feature
- **WHEN** a binding requests a non-`cpu` device on the `single` path
- **THEN** `try_load` returns the existing `UnsupportedModel` error verbatim ("the Candle LLM runtime supports `cpu` execution only")

#### Scenario: Multi-GPU topology is enumerated on a CUDA build
- **GIVEN** a build with the `candle-cuda` feature on a host with more than one CUDA device
- **WHEN** `discover_cluster_topology()` runs
- **THEN** it enumerates every available CUDA device (the enumeration loop is live once the candle CUDA backend is compiled in)
- **AND** per-device free-VRAM telemetry (NVML) and the NCCL all-reduce are validated on the CUDA CI lane as hardware-gated follow-ups (see `tasks.md` Tasks 5–6); the CPU-staged summation remains the numerically-equivalent reduction on every non-CUDA build

## Implementation status as of this change
This change wires the parallel engines delivered by `2026-06-19-distributed-model-parallel-inference` into the live model-load path and activates the candle CUDA *build* they target. Delivered and CPU-tested: tensor-parallel selection + full decode, pipeline-parallel selection + prefill (decode returns a typed error), expert-parallel validation + placement (load returns a typed error pending a full MoE loader), hardware-aware topology rejection before any weight load, and keying the dispatch/enumeration/all-reduce off the pre-existing `candle-cuda` feature (which makes `discover_cluster_topology`'s GPU enumeration live on a CUDA build; `nvfp4-cuda` stays CPU-buildable so the standard feature matrix keeps compiling).

Deferred, hardware-gated (cannot be compile-verified without a CUDA toolchain, so left for the CUDA CI lane #196/#197): per-device free-VRAM telemetry via NVML (`free_vram_bytes` is reported as `0`/unknown), and the NCCL all-reduce in `RowParallelLinear::forward` (still the CPU-staged summation, numerically identical and the path every CI test exercises). Known limits carried forward intentionally: pipeline parallelism is prefill-only (no decode-time per-stage KV cache) and its scheduler does not yet produce real wall-clock stage overlap; MoE routing is top-1 and has no full-model loader; ONNX/NVFP4 GPU forward passes remain the scope of `gpu-accelerated-inference-execution`.
