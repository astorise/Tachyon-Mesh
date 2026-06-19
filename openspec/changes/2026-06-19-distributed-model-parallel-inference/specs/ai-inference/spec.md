# ai-inference Delta

## ADDED Requirements

### Requirement: The runtime MUST execute tensor-parallel inference across multiple GPUs
When a model deployment is configured with `hardware_strategy.distribution_mode: tensor_parallelism` and `multi_gpu: true`, the inference runtime SHALL shard transformer layer weights across the configured GPU set and SHALL synchronize partial results between shards on every layer that requires it.

#### Scenario: A model exceeding single-GPU VRAM is sharded across GPUs
- **GIVEN** a model deployment configured with `distribution_mode: tensor_parallelism` and a GPU set whose combined VRAM, but not any single member's VRAM, can hold the model
- **WHEN** the model broker loads the model
- **THEN** the runtime partitions attention and MLP weights across the configured GPUs
- **AND** synchronizes partial activations across shards via an all-reduce/all-gather step per transformer block
- **AND** produces output numerically equivalent (within floating-point tolerance) to a single-GPU reference run of the same model on hardware where that reference fits

#### Scenario: Single-GPU deployments are unaffected
- **WHEN** a model deployment is configured with `distribution_mode: single` or `multi_gpu: false`
- **THEN** the runtime executes the existing single-device path unchanged
- **AND** no tensor-parallel synchronization code path is invoked

### Requirement: The runtime MUST execute pipeline-parallel inference across multiple nodes
When a model deployment is configured with `hardware_strategy.distribution_mode: pipeline_parallelism`, the runtime SHALL assign contiguous layer ranges to distinct nodes/GPUs and SHALL stream activations between pipeline stages over the existing mesh transport.

#### Scenario: Layers are split across pipeline stages
- **GIVEN** a model deployment configured with `distribution_mode: pipeline_parallelism` across N nodes
- **WHEN** the model broker loads the model
- **THEN** each node is assigned a contiguous, non-overlapping range of layers
- **AND** each node executes its layer range using the existing layer-wise streaming primitive
- **AND** activations are transmitted to the next stage over the mesh's existing gRPC/HTTP2 transport

#### Scenario: Pipeline depth bounds in-flight micro-batches
- **GIVEN** a pipeline-parallel deployment with a configured pipeline depth
- **WHEN** multiple inference requests are in flight concurrently
- **THEN** the scheduler admits at most the configured number of micro-batches into the pipeline at once
- **AND** additional requests queue rather than unboundedly growing per-stage memory usage

### Requirement: The runtime MUST execute expert-parallel inference for Mixture-of-Experts checkpoints
For checkpoints declaring expert tensors, the runtime SHALL place experts across the configured GPU/node set and SHALL route each token only to the device(s) hosting its selected expert(s), rather than replicating all experts on every device.

#### Scenario: MoE checkpoint experts are placed across devices
- **GIVEN** a model deployment configured with `distribution_mode: expert_parallelism` and a checkpoint declaring expert tensors
- **WHEN** the model broker loads the model
- **THEN** the runtime partitions experts across the configured device set
- **AND** does not load a full replica of every expert onto every device

#### Scenario: Tokens are routed only to their selected expert's device
- **WHEN** the gate layer selects the top-k experts for a token
- **THEN** the runtime forwards that token's hidden state only to the device(s) hosting the selected expert(s)
- **AND** non-MoE checkpoints continue to execute the existing dense path unchanged

### Requirement: Parallel execution plans MUST be validated against discovered hardware topology before deployment
The runtime SHALL reject, with a typed topology error, any `tensor_parallelism`, `pipeline_parallelism`, or `expert_parallelism` deployment whose GPU/node count, interconnect class, or per-shard VRAM requirement cannot be satisfied by the cluster's discovered hardware topology.

#### Scenario: Insufficient GPU count is rejected at deploy time
- **WHEN** a deployment requests `tensor_parallelism` across more GPUs than are available on the target node
- **THEN** `apply-model-deployment` fails with a typed `InsufficientGpuCount` error
- **AND** no partial model load is attempted

#### Scenario: Incompatible interconnect is rejected at deploy time
- **WHEN** a deployment requests `tensor_parallelism` across GPUs that lack the required high-bandwidth interconnect
- **THEN** `apply-model-deployment` fails with a typed `IncompatibleInterconnect` error

#### Scenario: Per-shard VRAM overrun is rejected at deploy time
- **WHEN** a deployment's computed per-shard VRAM requirement exceeds any target GPU's available VRAM
- **THEN** `apply-model-deployment` fails with a typed `VramPerShardExceeded` error
- **AND** the runtime does not silently downgrade to a single-GPU execution plan
