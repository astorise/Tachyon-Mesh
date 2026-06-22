# Implementation Tasks

- [ ] **Task 1: Carry `hardware-strategy` into `IntegrityModelBinding`**
  - Add a `HardwareStrategy` struct (`distribution_mode`, `device_ids`, `stage_layer_ranges`, `expert_device_map`, `pipeline_depth`) and a `hardware_strategy` field on `IntegrityModelBinding` in `core-host/src/host_core/domain_types.rs`, with `#[serde(default, skip_serializing_if = "HardwareStrategy::is_single")]` so existing configs and fixtures are byte-for-byte unaffected.
  - Map the already-validated WIT `hardware-strategy` record onto the emitted binding in `system-faas-config-api` (the strategy is currently validated structurally and then dropped).
  - Regression: a config with no `hardware_strategy` deserializes to `single` and re-serializes without the field.

- [ ] **Task 2: `LoadedModel::Parallel` variant + engine construction**
  - Add `LoadedModel::Parallel(ParallelModel)` and the `ParallelModel { Tensor, Pipeline, Expert }` enum in `candle_llm_runtime.rs`, boxing the engines.
  - Add `ParallelExecutionPlan::from_strategy(&HardwareStrategy)` and `resolve_devices(&[u32])` (CUDA ordinals under the CUDA feature, `Device::Cpu` otherwise).

- [ ] **Task 3: Dispatch in `try_load` + generation routing**
  - Thread a `&HardwareStrategy` parameter into `CandleLlmRuntime::try_load` and the call site in `core-host/src/ai_inference.rs:951` (the mock/ONNX/NVFP4 branches pass the default single strategy).
  - When `distribution_mode != single`: build the plan, run hardware-aware `validate_parallel_topology` against `discover_cluster_topology()`, resolve devices, construct the matching engine, return `LoadedModel::Parallel`.
  - Relax the line-258 device check to "reject GPU devices unless `candle-cuda` is compiled in"; the `single` path keeps the existing typed `UnsupportedModel` error verbatim on CUDA-less builds.
  - Add a `Parallel` arm to the generation/decode dispatch. Pipeline parallelism returns prompt logits (prefill) and a typed "decode not yet supported for pipeline parallelism" error for token streaming; tensor and expert parallelism support full decode.

- [ ] **Task 4: Activate the candle CUDA build**
  - Change `nvfp4-cuda = ["ai-inference"]` to `nvfp4-cuda = ["ai-inference", "candle-cuda"]` in `core-host/Cargo.toml` so the FP4 GPU build pulls the existing `candle-cuda` feature (`candle-core/cuda` et al.). Keep `default = ["ring"]` CUDA-free.
  - Confirm the default build, `--features ai-inference`, and `wasm32-wasip2` builds are unchanged (no CUDA toolchain required).

- [ ] **Task 5: Real multi-GPU enumeration + VRAM (NVML)**
  - Under `#[cfg(feature = "candle-cuda")]`, make `discover_cluster_topology()` enumerate all available CUDA ordinals and report real free VRAM per device via `nvml-wrapper` (new dep, gated on the CUDA feature). Read NVLink/PCIe interconnect class from NVML where available; default to `Pcie` when unknown.
  - Without the feature, the function reports exactly today's single-CPU topology (regression-guarded).

- [ ] **Task 6: NCCL all-reduce**
  - Under `#[cfg(feature = "candle-cuda")]` with >1 real device, replace the CPU-staged summation in `RowParallelLinear::forward` with an NCCL all-reduce. Keep the CPU summation for single-device, CPU, and CUDA-less builds.
  - The numeric contract is unchanged (NCCL sum == CPU sum within tolerance); the existing dense-equivalence tests remain the oracle.

- [ ] **Task 7: Tests**
  - Dispatch selection (CPU stand-ins, no CUDA needed): a `tensor_parallelism` binding loads `LoadedModel::Parallel(Tensor(..))` and produces logits equal (within `1e-3`) to the dense `Safetensors` path on the same tiny checkpoint; same for `pipeline_parallelism` (prefill) and `expert_parallelism`.
  - Topology rejection: a binding requesting more devices than discovered fails `try_load` with the typed topology error and loads no weights.
  - Regression: a `single` / absent-strategy binding loads the existing `Safetensors`/`Gguf` path unchanged and a GPU request on a CUDA-less build still returns the original `UnsupportedModel` error verbatim.
  - Hardware-gated lane (skipped without CUDA, run on the CI CUDA jobs #196/#197): NCCL all-reduce output matches the CPU-summation reference; `discover_cluster_topology()` reports >1 device with non-zero VRAM.

- [ ] **Task 8: Docs**
  - Update the `README.md` roadmap bullet and the `wit/config-ai.wit` `gpu-distribution` doc comment (both currently say "no deployment wires `hardware-strategy` into model loading yet") to state that the dispatch path now selects the parallel engines, while documenting the remaining P1 gaps (pipeline decode KV-cache, threaded stage overlap) honestly.
  - Update `CHANGELOG.md` with the runtime now selecting TP/PP/MoE engines and the `nvfp4-cuda` → `candle-cuda` activation.
