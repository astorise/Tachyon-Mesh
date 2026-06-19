# Implementation Tasks

- [x] **Task 1: WIT contract**
  - Add `parallel-strategy`, `parallel-execution-plan`, and `validate-parallel-topology` to `wit/ai/inference.wit` as defined in `design.md`.
  - Regenerate host/guest bindings. *(No-op: `wit/ai/inference.wit` is not currently consumed by any `wit_bindgen::generate!` call site in the tree — confirmed by grep — so there are no generated bindings to regenerate. This mirrors the rest of `inference.wit`, which is documented as unwired v1.2 scaffolding.)*

- [x] **Task 2: Topology discovery**
  - Added `discover_cluster_topology()` in `core-host/src/ai_inference/parallel.rs`. Always reports CPU device 0; under the `nvfp4-cuda` feature it probes additional CUDA ordinals via `candle_core::Device::cuda_if_available` and stops at the first unavailable ordinal.
  - **Caveat**: `candle-core`'s own `cuda` Cargo feature is not enabled by `nvfp4-cuda` in `core-host/Cargo.toml` (confirmed — `nvfp4-cuda = ["ai-inference"]` only), so `cuda_is_available()` is always `false` in the current build and this function reports a single CPU device today. Per-GPU free VRAM is reported as `0` (unknown) on every device, since neither `candle_core` nor this crate bind NVML/`cudaMemGetInfo`. Real multi-GPU enumeration and VRAM telemetry depend on the CUDA backend wiring tracked by `2026-06-19-gpu-accelerated-inference-execution`; interconnect class (NVLink vs. PCIe) detection is not implemented and conservatively defaults to `Pcie` whenever more than one device is discovered.

- [x] **Task 3: Topology validation**
  - `validate_parallel_topology` (hardware-aware: device count, interconnect class, VRAM-per-shard) is implemented and unit-tested in the new shared `crates/parallel-topology` crate, re-exported by `core-host/src/ai_inference/parallel.rs` for use once the dispatch path (Tasks 4-6) lands.
  - Extended `wit/config-ai.wit`'s `hardware-strategy` record with `device-ids`, `stage-layer-ranges`, `expert-device-map`, and `pipeline-depth`, and added the missing `expert-parallelism` variant to `gpu-distribution`, so a deployment now carries enough information to describe a real parallel plan (previously only `multi-gpu: bool` + `distribution-mode`).
  - **Pre-existing build defect fixed as a prerequisite**: `system-faas-config-api` (the only crate implementing `apply-model-deployment`) failed to build for `wasm32-wasip2` — confirmed pre-existing and unrelated to this change via `git stash`. Root cause: 13 `wit_bindgen::generate!` calls in one crate (one per config domain), each embedding its own world's export requirements, with only 1 `export!()` call total for the unrelated routing/handler world. Fixed by adding the `stubs` keyword (auto-generates an `unreachable!()`-bodied `Stub` + `export!(Stub)`) to the 12 config domains that remain scaffold-only, while wiring `ai_contract` for real.
  - Added `AiConfigComponent` implementing `ai_contract::exports::tachyon::ai_config::config_ai::Guest`. `validate-ai-config` and `apply-model-deployment` both map each deployment's `hardware-strategy` into a `parallel_topology::ParallelExecutionPlan` and call `parallel_topology::validate_plan_shape` — the *structural* (device-id count, pipeline stage contiguity/coverage, expert-device-map bounds) check, since a Wasm guest component has no access to live hardware topology. `get-ai-config` returns an explicit "not wired" error rather than fabricating a config store. Hardware-aware validation (`validate_parallel_topology`, run against real discovered devices) remains `core-host`'s responsibility once Tasks 4-6 build the dispatch path that actually admits a deployment for execution.
  - 5 new unit tests cover single/tensor/pipeline/expert strategies through `validate_hardware_strategy`, plus the pre-existing 17 scaffold tests. `cargo build -p system-faas-config-api --target wasm32-wasip2` now succeeds.

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
