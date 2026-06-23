# Implementation Tasks

- [x] **Task 1: NCCL dependency and communicator lifecycle**
  - Added `cudarc = { version = "0.19.7", default-features = false, features = ["nccl"], optional = true }` to `core-host/Cargo.toml`, gated behind `candle-cuda` (`dep:cudarc`) — same version `candle-core`'s CUDA backend already vendors, so no second NCCL binding.
  - Added `NcclShardGroup` (`parallel.rs`): one communicator per device via `Comm::from_devices` (`ncclCommInitAll`'s single-process path), built once in `TensorParallelLlama::load` and threaded as `Arc<NcclShardGroup>` through `TensorParallelBlock`/`TensorParallelMlp`/`RowParallelLinear` (mirrors `TensorParallelCache`).

- [x] **Task 2: Real NCCL all-reduce in `RowParallelLinear::forward`**
  - Implemented the dispatch from `design.md` §2.3 in `RowParallelLinear::all_reduce`: real NCCL `AllReduce` when an `NcclShardGroup` is attached and every partial is a contiguous CUDA `DType::F32` tensor with >1 device participating; the original host-staged manual sum (`cpu_staged_sum`, byte-for-byte unchanged) otherwise.
  - Verified: `cargo test -p core-host --features ai-inference ai_inference::` — 96/96 passed, including `row_parallel_all_reduce_matches_single_device_reference`, with zero regressions.

- [x] **Task 3: NVML VRAM telemetry**
  - Added `nvml-wrapper = { version = "0.12", optional = true }` to `core-host/Cargo.toml`, gated behind `candle-cuda`.
  - Implemented `free_vram_bytes(ordinal)` in `parallel.rs` exactly per `design.md` §3.2 (`OnceLock<Option<Nvml>>`, `Nvml::init().ok()`) and wired it into `discover_cluster_topology`'s CUDA-enumeration loop, replacing the hardcoded `0`.
  - `Nvml::init()` failure degrades to `0` via `.ok()`/`unwrap_or(0)` — no panic path exists in this function; not separately unit-testable without real NVML present, so this is documented rather than covered by a forced-failure test.

- [x] **Task 4: Fix the verified `nvfp4-cuda`/`candle-cuda` comment bug**
  - Corrected the comment above `discover_cluster_topology`'s CUDA-enumeration loop in `core-host/src/ai_inference/parallel.rs` per `design.md` §3.3, removing the inaccurate "pulled by `nvfp4-cuda`" claim.
  - Done as part of resolving the `origin/main` merge conflict in this file.

- [x] **Task 5: GPU-proof CI step**
  - Added `nccl_all_reduce_matches_cpu_staged_reference` to `parallel.rs`, gated `#[cfg(feature = "candle-cuda")]`, using two `ncclCommInitAll` loopback ranks on the one available CUDA device (§2.4); skips (does not fail) if no CUDA device is reachable.
  - Added the `cargo test -p core-host --features candle-cuda nccl_all_reduce_matches_cpu_staged_reference -- --nocapture` step to the `cuda-quality` job in `.github/workflows/ci.yml`, after the existing `cargo clippy --features candle-cuda` step.
  - **Not yet independently re-verified against a live `arc-gpu-runners` run** — this sandbox has no CUDA toolchain (`nvcc` not found), so `--features candle-cuda` cannot be compiled or executed locally. The next push that triggers `cuda-quality` on the real runner is the actual proof; confirm via `mcp__github__get_job_logs` once it runs.

- [~] **Task 6: Tests**
  - `cargo test -p core-host --features ai-inference parallel::` — done, 0 regressions (18/18 passed).
  - `cargo test -p core-host --features candle-cuda ...` on real CUDA hardware — **not run locally** (no CUDA toolchain in this sandbox); pending the `cuda-quality` CI run.
  - `cargo clippy -p core-host --features candle-cuda ...` — **not run locally** for the same reason; `cargo clippy -p core-host --features ai-inference --all-targets -- -D warnings -D clippy::unwrap_used` (the non-CUDA build) was run and is clean.

- [x] **Task 7: Docs**
  - Updated this change's `specs/ai-inference/spec.md` delta below with an "Implementation status" note distinguishing the now-real NCCL/NVML paths from the CPU-staged fallback.
  - Added a README/CHANGELOG note stating tensor-parallel all-reduce now uses real NCCL on CUDA builds with ≥2 GPUs, and VRAM-aware topology validation is now enforceable on real hardware via NVML.
