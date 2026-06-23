# ai-inference Delta

## MODIFIED Requirements

### Requirement: The runtime MUST execute tensor-parallel inference across multiple GPUs
When a model deployment is configured with `hardware_strategy.distribution_mode: tensor_parallelism` and `multi_gpu: true`, the inference runtime SHALL shard transformer layer weights across the configured GPU set and SHALL synchronize partial results between shards on every layer that requires it using a real collective-communication primitive on CUDA hardware, falling back to a host-staged reduction only when no CUDA backend with multiple participating GPUs is available.

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

#### Scenario: A real NCCL collective performs the all-reduce on CUDA hardware
- **GIVEN** the runtime is built with the `candle-cuda` feature and a tensor-parallel shard group spans 2 or more CUDA devices
- **WHEN** `RowParallelLinear::forward` synchronizes partial outputs across the shard group
- **THEN** the runtime issues a real NCCL `AllReduce` collective across the participating devices' communicators
- **AND** the reduced result matches the existing host-staged-sum reference within `1e-4` tolerance

#### Scenario: The host-staged fallback remains correct when no multi-GPU CUDA group is available
- **GIVEN** the runtime is built without the `candle-cuda` feature, or a shard group has fewer than 2 CUDA devices, or the shard group runs on `Device::Cpu`
- **WHEN** `RowParallelLinear::forward` synchronizes partial outputs
- **THEN** the runtime performs the existing host-staged manual sum across devices
- **AND** the result is unchanged from the pre-existing behavior, with no regression in any CPU-only test

### Requirement: Parallel execution plans MUST be validated against discovered hardware topology before deployment
The runtime SHALL reject, with a typed topology error, any `tensor_parallelism`, `pipeline_parallelism`, or `expert_parallelism` deployment whose GPU/node count, interconnect class, or per-shard VRAM requirement cannot be satisfied by the cluster's discovered hardware topology. On CUDA builds, per-device free VRAM SHALL be sourced from real NVML telemetry rather than a hardcoded placeholder value, so the VRAM check can actually reject an oversized deployment in production.

#### Scenario: Insufficient GPU count is rejected at deploy time
- **WHEN** a deployment requests `tensor_parallelism` across more GPUs than are available on the target node
- **THEN** `apply-model-deployment` fails with a typed `InsufficientDeviceCount` error
- **AND** no partial model load is attempted

#### Scenario: Incompatible interconnect is rejected at deploy time
- **WHEN** a deployment requests `tensor_parallelism` across GPUs that lack the required high-bandwidth interconnect
- **THEN** `apply-model-deployment` fails with a typed `IncompatibleInterconnect` error

#### Scenario: Per-shard VRAM overrun is rejected at deploy time using real telemetry
- **GIVEN** the runtime is built with the `candle-cuda` feature and NVML successfully reports each CUDA device's free VRAM
- **WHEN** a deployment's computed per-shard VRAM requirement exceeds any target GPU's NVML-reported free VRAM
- **THEN** `apply-model-deployment` fails with a typed `VramPerShardExceeded` error
- **AND** the runtime does not silently downgrade to a single-GPU execution plan

#### Scenario: VRAM telemetry degrades gracefully when NVML is unavailable
- **GIVEN** NVML initialization fails (no NVIDIA driver, insufficient permissions, or a non-NVIDIA host) or the `candle-cuda` feature is not compiled in
- **WHEN** the runtime discovers cluster topology
- **THEN** every device reports `free_vram_bytes: 0` ("unknown"), matching the existing pre-NVML behavior
- **AND** `validate_parallel_topology` never rejects a deployment on VRAM grounds for a device reporting `0`

## ADDED Requirements

### Requirement: CUDA CI MUST prove multi-GPU collective execution, not just compilation
The `cuda-quality` CI job (or an equivalent job on the same GPU-equipped self-hosted runner) SHALL execute a test that exercises a real NCCL all-reduce on real CUDA hardware and asserts its numeric result against a known-correct reference, in addition to the existing `cargo check`/`cargo clippy --features candle-cuda` compilation/lint steps.

#### Scenario: GPU CI runs and passes a real NCCL all-reduce test
- **GIVEN** the `cuda-quality` job runs on the `arc-gpu-runners` self-hosted runner with a real GPU detected via `nvidia-smi`
- **WHEN** the job executes its test step
- **THEN** a test exercising `ncclAllReduce` across multiple ranks runs to completion
- **AND** its result matches the existing CPU-staged-sum reference within `1e-4` tolerance
- **AND** the job's overall conclusion is `success` only if that test passes, not merely if `cargo clippy` finds no lint errors

#### Scenario: The NCCL test runs correctly on a single-physical-GPU runner
- **GIVEN** the runner exposes exactly one physical CUDA device (the verified case for the current `arc-gpu-runners` configuration)
- **WHEN** the NCCL all-reduce test runs
- **THEN** it uses multiple NCCL ranks on that single device (loopback communicator initialization) rather than requiring a second physical GPU
- **AND** the test is skipped, not failed, on a `candle-cuda` build executed on a host reporting zero CUDA devices

### Requirement: The `nvfp4-cuda` and `candle-cuda` Cargo features MUST be documented as independent
Inline documentation describing the relationship between the `nvfp4-cuda` and `candle-cuda` Cargo features SHALL accurately reflect that they are separate, sibling features — enabling one does not enable the other — matching `core-host/Cargo.toml`'s actual feature graph.

#### Scenario: The topology-discovery comment accurately describes feature independence
- **GIVEN** a reader inspects the comment above `discover_cluster_topology`'s CUDA-enumeration loop in `core-host/src/ai_inference/parallel.rs`
- **WHEN** they read the comment to understand what enables multi-GPU enumeration
- **THEN** the comment states that the `candle-cuda` feature, not `nvfp4-cuda`, is required
- **AND** the comment does not claim `nvfp4-cuda` pulls in or implies `candle-cuda`

## Implementation status as of this change

`RowParallelLinear::all_reduce` (`core-host/src/ai_inference/parallel.rs`) now dispatches to a
real `cudarc::nccl::Comm::all_reduce` collective, via the new `NcclShardGroup` (one communicator
per device, built once per `TensorParallelLlama::load` call and shared as `Arc<NcclShardGroup>`
across every layer), whenever the runtime is built with `candle-cuda`, the shard group spans
2+ CUDA devices, and every partial is a contiguous `DType::F32` CUDA tensor. The pre-existing
host-staged `cpu_staged_sum` path is unchanged byte-for-byte and remains the fallback in every
other case (CPU-only builds, single-device groups, non-F32 dtypes, non-contiguous tensors).
`discover_cluster_topology`'s per-device `free_vram_bytes` is now sourced from real NVML
telemetry (`Nvml::device_by_index(..).memory_info().free`) on `candle-cuda` builds, degrading
to the pre-existing `0` ("unknown") on NVML init failure or non-CUDA builds — `0` is still never
treated as a deployable amount of VRAM by `validate_parallel_topology`.

**Locally verified**: the default `ai-inference` (non-CUDA) build and its full test suite
(96/96 `ai_inference::` tests, including `row_parallel_all_reduce_matches_single_device_reference`)
plus `clippy -D warnings -D clippy::unwrap_used` are unaffected — zero regressions. `cargo check
--features candle-cuda` resolves the new `cudarc`/`nvml-wrapper` dependency graph successfully
and fails only at `cudarc`'s build script's `nvcc` toolchain check, confirming this sandbox's
absence of CUDA, not a code defect.

**Not yet independently verified on real hardware**: this sandbox has no CUDA toolchain, so the
new `nccl_all_reduce_matches_cpu_staged_reference` test and the `candle-cuda` clippy lint have
not been run locally. The added `cuda-quality` CI step on `arc-gpu-runners` is the actual proof
of this change's GPU-execution requirements; see `tasks.md` Task 5/6 for status and how to
confirm it via `mcp__github__get_job_logs` once it runs.
