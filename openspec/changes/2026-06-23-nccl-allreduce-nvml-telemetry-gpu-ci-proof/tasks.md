# Implementation Tasks

- [ ] **Task 1: NCCL dependency and communicator lifecycle**
  - Add `cudarc`'s NCCL bindings (or `nccl-sys`, whichever avoids a second NCCL binding alongside `candle_core`'s existing CUDA backend dependency) to `core-host/Cargo.toml`, gated behind the existing `candle-cuda` feature only — no new feature flag.
  - Add `NcclShardGroup` (per `design.md` §2.2): one NCCL communicator per participating CUDA device, created once per tensor-parallel shard group and threaded through `TensorParallelBlock`/`RowParallelLinear` the same way `TensorParallelCache` is already threaded through layers.

- [ ] **Task 2: Real NCCL all-reduce in `RowParallelLinear::forward`**
  - Implement the dispatch in `design.md` §2.3: real NCCL `AllReduce` when `candle-cuda` is active and ≥2 CUDA devices participate; fall back to the existing host-staged manual sum otherwise (no-CUDA build, single device, or `Device::Cpu`).
  - Verify the existing CPU-path tests (`row_parallel_all_reduce_matches_single_device_reference` and friends) are unaffected — they must continue to pass unchanged, proving the fallback branch's behavior is preserved exactly.

- [ ] **Task 3: NVML VRAM telemetry**
  - Add `nvml-wrapper` to `core-host/Cargo.toml`, gated behind `candle-cuda`.
  - Implement `free_vram_bytes(ordinal)` (per `design.md` §3.2) and call it from `discover_cluster_topology` for each enumerated CUDA device, replacing the hardcoded `0`.
  - Confirm `Nvml::init()` failure (no driver, no permissions, non-NVIDIA host) degrades to `0` without panicking or failing the build — add a unit test that simulates/forces the `None` path if the existing code structure allows injecting it, otherwise document the manual verification performed.

- [ ] **Task 4: Fix the verified `nvfp4-cuda`/`candle-cuda` comment bug**
  - Correct the comment above `discover_cluster_topology`'s CUDA-enumeration loop in `core-host/src/ai_inference/parallel.rs` per `design.md` §3.3, removing the inaccurate "pulled by `nvfp4-cuda`" claim.

- [ ] **Task 5: GPU-proof CI step**
  - Add the new NCCL all-reduce test described in `design.md` §4.2 to `parallel.rs`, gated `#[cfg(feature = "candle-cuda")]`, using `ncclCommInitAll` loopback ranks (§2.4) so it runs correctly on the verified single-GPU `arc-gpu-runners` runner.
  - Add the `cargo test -p core-host --features candle-cuda nccl_all_reduce_matches_cpu_staged_reference` step to the `cuda-quality` job in `.github/workflows/ci.yml`, after the existing `cargo clippy --features candle-cuda` step.
  - Confirm the job still passes end-to-end on the real `arc-gpu-runners` runner (this is the "GPU CI proof" deliverable — re-verify via the same `mcp__github__get_job_logs` method used to confirm the pre-change `cuda-quality` baseline, looking for the new test's `PASSED`/`ok` output, not just a clean `cargo clippy` finish).

- [ ] **Task 6: Tests**
  - `cargo test -p core-host --features ai-inference parallel::` (CPU-only fallback paths) must show 0 regressions.
  - `cargo test -p core-host --features candle-cuda` on a CUDA-capable host must show the new NCCL all-reduce test passing in addition to all pre-existing tests.
  - `cargo clippy -p core-host --features candle-cuda --all-targets -- -D warnings -D clippy::unwrap_used` must remain clean (matching the already-verified pre-change baseline).

- [ ] **Task 7: Docs**
  - Update this change's `specs/ai-inference/spec.md` delta's "Implementation status" note once code lands, distinguishing the now-real NCCL/NVML paths from the CPU-staged fallback that remains for non-CUDA builds.
  - Add a README/CHANGELOG note (matching the existing convention from `2026-06-19-distributed-model-parallel-inference`'s Task 8) stating that tensor-parallel all-reduce now uses real NCCL on CUDA builds with ≥2 GPUs, and that VRAM-aware topology validation is now enforceable on real hardware via NVML.
