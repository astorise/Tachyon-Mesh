# Implementation Tasks

- [x] **Task 1: Carry `hardware-strategy` into `IntegrityModelBinding`**
  - Added `GpuDistribution` (single/tensor/pipeline/expert) and `HardwareStrategy` (`distribution_mode`, `device_ids`, `stage_layer_ranges`, `expert_device_map`, `pipeline_depth`) to `core-host/src/host_core/domain_types.rs`, plus a `hardware_strategy` field on `IntegrityModelBinding` with `#[serde(default, skip_serializing_if = "HardwareStrategy::is_single")]`.
  - All ~29 existing binding literals (production + integration tests) updated with `hardware_strategy: Default::default()`.
  - Verified by `hardware_strategy_tests` (3 tests): a legacy binding with no field deserializes to `single`; a default strategy is omitted from serialized output; a tensor-parallel strategy round-trips.
  - **Note**: mapping the WIT `hardware-strategy` onto the emitted binding in `system-faas-config-api` was *not* changed here — that crate validates the plan shape in the Wasm guest and does not itself construct `IntegrityModelBinding` (the host-side `core-host` config path does). The host binding now carries the field; the config-api guest already validates plan shape (distributed change, Task 3).

- [x] **Task 2: `LoadedModel::Parallel` variant + engine construction**
  - Added `LoadedModel::Parallel(ParallelModel)` with `ParallelModel { Tensor, Pipeline }` (boxed engines via the enum). `load_parallel` + `plan_from_strategy` + `resolve_devices` + `load_llama_config` added to `candle_llm_runtime.rs`.
  - **Scope correction (no `Expert` variant)**: there is no full MoE model in the tree — only the verified per-layer `ExpertParallelMlp` primitive — so an `expert_parallelism` strategy is validated and device-placed but returns a typed `UnsupportedModel` error at load ("requires an MoE checkpoint loader, which is not yet implemented") rather than constructing a non-existent full model. A `ParallelModel::Expert` variant is intentionally omitted until a Mixtral-style loader lands.

- [x] **Task 3: Dispatch in `try_load` + generation routing**
  - `try_load` gains a `&HardwareStrategy` parameter (call site in `core-host/src/ai_inference.rs:952` passes `&binding.hardware_strategy`). Factored a `try_load_with_topology` inner fn so tests can inject a `ClusterTopology` (production calls `discover_cluster_topology()`).
  - When `distribution_mode != single`: builds the plan, runs hardware-aware `validate_parallel_topology` *before loading weights*, resolves devices, constructs `TensorParallelLlama` or `PipelineParallelLlama`.
  - Line-258 device check is now `strategy.is_single() && requested_device != "cpu"` — the single dense path remains CPU-only and rejects a GPU request verbatim; the parallel path resolves its devices from the validated plan instead.
  - Generation routing: `decode` gains a `Parallel` arm. Tensor parallelism drives the full `decode_loop` with a `TensorParallelCache` (decode_loop now takes an `input_device` so the input tensor lands on the engine's primary device). Pipeline parallelism returns a typed `Execution` error ("prefill-only, pending a per-stage KV cache across decode steps") rather than producing wrong output.

- [x] **Task 4: Activate the candle CUDA build**
  - The candle CUDA backend is activated by the pre-existing `candle-cuda` feature (`candle-core/cuda` + friends), which the dispatch, enumeration, and all-reduce gate on (`#[cfg(feature = "candle-cuda")]`). `default = ["ring"]` stays CUDA-free.
  - **Correction (CI)**: an initial attempt wired `nvfp4-cuda = ["ai-inference", "candle-cuda"]`, but that broke the standard feature matrix — CI builds an all-features combo (including `nvfp4-cuda`) on a non-CUDA runner, and pulling `candle-core/cuda` drags in `cudarc`, whose build script requires `nvcc`. Reverted: `nvfp4-cuda` stays `["ai-inference"]` (CPU-buildable, remains in the matrix); CUDA lives entirely behind `candle-cuda`, exercised by the dedicated `cuda-quality` job.
  - Verified the default build, the `ai-inference` build/tests, and the full feature matrix compile without a CUDA toolchain; `cuda-quality` (clippy `--features candle-cuda`) is green on the CUDA lane.

- [x] **Task 5: Real multi-GPU enumeration + VRAM (NVML)**
  - **Enumeration (done)**: `discover_cluster_topology()`'s CUDA-ordinal enumeration loop is re-gated from `nvfp4-cuda` to `candle-cuda`; with Task 4's wire, `cuda_if_available` now actually opens devices, so the loop enumerates every real GPU on a `candle-cuda` build (it reported a single CPU device before only because the backend was never compiled in). `resolve_devices` maps plan device IDs to CUDA ordinals on that build, CPU stand-ins otherwise.
  - **VRAM via NVML (done)**: `free_vram_bytes` is populated through `nvml-wrapper` on `candle-cuda` builds, degrading safely to `0` when NVML is unavailable.

- [x] **Task 6: NCCL all-reduce**
  - `RowParallelLinear::forward` uses a shared `NcclShardGroup` and real `cudarc::nccl::Comm::all_reduce` on multi-GPU `candle-cuda` builds, retaining the CPU-staged sum as the fallback.
  - The hardware-gated CI proof requires two distinct GPUs. This workstation has one RTX 3070 Ti; its local CUDA build also fails in Candle's kernel build because CUDA 13.2 rejects the installed Windows host compiler. The dedicated `cuda-quality` runner remains the authoritative two-GPU proof lane.

- [x] **Task 7: Tests**
  - `tensor_parallel_strategy_dispatches_and_matches_the_dense_runtime`: a `tensor_parallelism` binding loads `LoadedModel::Parallel(Tensor)`, its prefill logits match the dense runtime within `1e-3`, and full generation runs the decode loop.
  - `pipeline_parallel_strategy_matches_dense_prefill_and_refuses_decode`: a `pipeline_parallelism` binding loads `LoadedModel::Parallel(Pipeline)`, prefill logits match the dense runtime within `1e-3`, and `generate` returns the typed prefill-only error.
  - `expert_parallel_strategy_is_rejected_until_a_moe_loader_exists`: typed `UnsupportedModel` error mentioning MoE.
  - `a_parallel_plan_exceeding_discovered_devices_is_rejected_before_loading`: a 2-device plan against a 1-device topology fails with the typed topology error and loads no weights.
  - `single_strategy_still_rejects_a_gpu_device_request`: the dense path still returns the verbatim "cpu execution only" error for a GPU request.
  - Plus the 3 `hardware_strategy_tests` serde regressions. All run on `Device::Cpu` stand-ins (no CUDA in CI), injecting a multi-device topology. Full suite: `core-host --features ai-inference ai_inference::` = 96 passed, 0 failed; `parallel-topology` = 13 passed.

- [x] **Task 8: Docs**
  - `wit/config-ai.wit` `gpu-distribution` comment rewritten: the strategy is now wired into model loading, with the per-mode status (tensor full decode / pipeline prefill-only / expert pending MoE loader) and the CUDA-lane caveat stated plainly.
  - `README.md` roadmap updated: a new bullet marks the dispatch path as landed and documents the remaining follow-ups (pipeline decode, MoE loader, CUDA/NCCL).
  - `CHANGELOG.md` "Unreleased" gains the dispatch + `candle-cuda` activation entry.
