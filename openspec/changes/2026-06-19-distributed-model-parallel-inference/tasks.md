# Implementation Tasks

- [ ] **Task 1: WIT contract**
  - Add `parallel-strategy`, `parallel-execution-plan`, and `validate-parallel-topology` to `wit/ai/inference.wit` as defined in `design.md`.
  - Regenerate host/guest bindings.

- [ ] **Task 2: Topology discovery**
  - Extend the existing hardware capability discovery (consumed by `hardware-capabilities`/`heterogeneous-accelerator-orchestration`) to report interconnect class (NVLink/PCIe/cross-node network) and per-GPU free VRAM, needed to validate a `parallel-execution-plan`.

- [ ] **Task 3: Topology validation**
  - Implement `validate-parallel-topology` in `system-faas-model-broker`, wired into the existing `apply-model-deployment` path from `config-ai.wit`.
  - Return typed `TopologyError` variants (insufficient GPU count, incompatible interconnect, per-shard VRAM exceeded) instead of silently downgrading to single-GPU.

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
