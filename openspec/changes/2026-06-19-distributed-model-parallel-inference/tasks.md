# Implementation Tasks

- [x] **Task 1: WIT contract**
  - Add `parallel-strategy`, `parallel-execution-plan`, and `validate-parallel-topology` to `wit/ai/inference.wit` as defined in `design.md`.
  - Regenerate host/guest bindings. *(No-op: `wit/ai/inference.wit` is not currently consumed by any `wit_bindgen::generate!` call site in the tree — confirmed by grep — so there are no generated bindings to regenerate. This mirrors the rest of `inference.wit`, which is documented as unwired v1.2 scaffolding.)*

- [x] **Task 2: Topology discovery**
  - Added `discover_cluster_topology()` in `core-host/src/ai_inference/parallel.rs`. Always reports CPU device 0; under the `nvfp4-cuda` feature it probes additional CUDA ordinals via `candle_core::Device::cuda_if_available` and stops at the first unavailable ordinal.
  - **Caveat**: `candle-core`'s own `cuda` Cargo feature is not enabled by `nvfp4-cuda` in `core-host/Cargo.toml` (confirmed — `nvfp4-cuda = ["ai-inference"]` only), so `cuda_is_available()` is always `false` in the current build and this function reports a single CPU device today. Per-GPU free VRAM is reported as `0` (unknown) on every device, since neither `candle_core` nor this crate bind NVML/`cudaMemGetInfo`. Real multi-GPU enumeration and VRAM telemetry depend on the CUDA backend wiring tracked by `2026-06-19-gpu-accelerated-inference-execution`; interconnect class (NVLink vs. PCIe) detection is not implemented and conservatively defaults to `Pcie` whenever more than one device is discovered.

- [ ] **Task 3: Topology validation**
  - `validate_parallel_topology` is implemented and unit-tested in `core-host/src/ai_inference/parallel.rs` (the real enforcement logic with typed `TopologyError` variants).
  - **Blocked/deferred, documented finding**: there is no existing host-side call path that invokes a guest's `apply-model-deployment` export — grepping `core-host` for `model_deployment`/`ModelDeployment`/`hardware_strategy` returns zero results. The only code that implements `apply-model-deployment` today is `systems/system-faas-config-api/src/lib.rs`'s `pub fn apply_model_deployment<T>(_deployment: T) -> Result<(), String> { Ok(()) }`, which is a generic, type-erased stub identical in shape to ~30 other unwired config-domain functions in that file (none of them call the real `wit_bindgen`-generated types from `ai_contract`, which is marked `#[allow(dead_code)]`). Additionally, `config-ai.wit`'s `hardware-strategy` record (`multi-gpu: bool`, `distribution-mode: gpu-distribution`) does not carry the device-ids/VRAM/layer-range fields `validate-parallel-topology` needs, so wiring this "for real" requires either (a) extending `config-ai.wit` to carry a `parallel-execution-plan`-shaped payload, or (b) adding a host-side deployment-admission path in `core-host` that doesn't exist yet. Recommend splitting this into its own follow-up task/change once the deployment-admission call path itself is built, rather than special-casing one of ~30 identical stubs in `system-faas-config-api` with logic it cannot actually exercise end-to-end.

- [ ] **Task 4: Tensor-parallel execution**
  - Implement column/row-parallel weight sharding for attention and MLP blocks in the Candle execution path.
  - Implement the all-reduce/all-gather synchronization point per transformer block (NCCL when `nvfp4-cuda`/CUDA is enabled; CPU-staged reduce otherwise for correctness on non-CUDA builds).
  - Gate entirely behind `multi_gpu: true` + `distribution_mode: tensor_parallelism`; single-GPU path must be byte-for-byte unaffected.

- [ ] **Task 5: Pipeline-parallel execution**
  - Reuse `ai-layer-wise-inference`'s per-layer streaming primitive as the per-stage executor.
  - Implement cross-node activation transport over the existing `grpc-http2` mesh transport.
  - Implement bounded-depth micro-batch scheduling across stages (configurable pipeline depth).

- [ ] **Task 6: Expert-parallel (MoE) execution**
  - Detect MoE checkpoints (expert tensor naming) at load time and build an `ExpertPlacementPlan` across the configured device set.
  - Implement gate-then-route token dispatch to the device hosting the selected expert(s), avoiding dense replication.
  - Fall back to the existing dense path for non-MoE checkpoints (no behavior change).

- [ ] **Task 7: Tests**
  - Multi-GPU-in-CI-container or mocked-`Device` unit tests for shard partitioning correctness (numeric equivalence vs. single-GPU reference for a small model).
  - Topology validation tests: insufficient GPU count, incompatible interconnect, VRAM-per-shard exceeded all produce typed errors and reject deployment.
  - Regression test: `multi_gpu: false` / `distribution_mode: single` deployments produce identical output to the pre-change engine.

- [ ] **Task 8: Docs**
  - Update `README.md` roadmap and `ai-orchestration` operator docs to reflect that `tensor_parallelism`/`pipeline_parallelism` are now executable, not configuration-only.
