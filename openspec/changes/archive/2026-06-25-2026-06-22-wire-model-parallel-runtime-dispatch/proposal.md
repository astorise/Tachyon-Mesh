# Proposal: Wire the Model-Parallel Engines into the Production Runtime + Activate Candle CUDA

## Why
The archived-in-spirit-but-still-active change `2026-06-19-distributed-model-parallel-inference` (8/8 tasks) shipped **real, numerically-verified** tensor-parallel, pipeline-parallel, and expert-parallel (MoE) engines (`core-host/src/ai_inference/tensor_parallel_llama.rs`, `pipeline_parallel_llama.rs`, `parallel.rs`) plus the shared `crates/parallel-topology` validator. Every one of those tasks closes with the same honest caveat: **nothing in production selects these engines.** Concretely, today:

- `candle_llm_runtime.rs:258` hard-rejects any `requested_device != "cpu"` (`"the Candle LLM runtime supports cpu execution only"`), so a deployment asking for a GPU never even reaches a parallel path.
- `LoadedModel` (the runtime's loaded-model enum) has only `Safetensors` and `Gguf` variants — no tensor/pipeline/expert variant exists for a dispatcher to select.
- `IntegrityModelBinding` (the runtime-side deployment config in `core-host/src/host_core/domain_types.rs`) carries `alias`, `path`, `device`, `qos`, `dynamic` — but **not** the `hardware-strategy` (distribution mode, device IDs, stage ranges, expert map). The WIT surface (`wit/config-ai.wit` `hardware-strategy`, `wit/ai/inference.wit` `parallel-execution-plan`) already defines these fields; they are simply never threaded from config into the loader.
- The build never enables candle's CUDA backend in the default or `nvfp4-cuda` configuration, so `cuda_is_available()` is `false`, every "multi-device" test uses `Device::Cpu` stand-ins, and the all-reduce in `RowParallelLinear` is a CPU summation rather than NCCL.

The result: an excellent, well-tested library that has **never run on a GPU and is unreachable from any deployment**. This change supplies the missing dispatch layer and the CUDA build activation — the chaînon manquant that turns the existing primitives into a usable feature. It is the P0 from the 2026-06-22 audit, and it is explicitly *out of scope* of the four existing change proposals (each of which assumes the wiring lands separately).

## What Changes
1. **Plumb `hardware-strategy` into the runtime config.** Extend `IntegrityModelBinding` (and the config path that populates it from `apply-model-deployment`) with the already-WIT-defined `distribution_mode`, `device_ids`, `stage_layer_ranges`, `expert_device_map`, and `pipeline_depth`. Default (`single`, empty lists) preserves today's behaviour byte-for-byte.
2. **Add a `Parallel` variant to `LoadedModel` and a dispatch in `try_load`.** When `distribution_mode != single`, after validating the plan against discovered hardware via `parallel_topology::validate_parallel_topology`, build the matching engine — `TensorParallelLlama`, `PipelineParallelLlama`, or `ExpertParallelMlp` — and serve generation through it. When `distribution_mode == single` (or strategy is absent), the existing `Safetensors`/`Gguf` path runs unchanged. The non-cpu rejection at line 258 is relaxed to: accept GPU devices *only* when the CUDA backend is compiled in; otherwise keep the typed error.
3. **Activate the candle CUDA build.** Make `nvfp4-cuda` (and a clearly-named umbrella feature) pull in the **already-existing** `candle-cuda` feature (`candle-core/cuda` + friends) so `cuda_is_available()` can be true; add real multi-GPU enumeration and per-device free-VRAM via NVML in `discover_cluster_topology()`; and replace the CPU-sum all-reduce in `RowParallelLinear::forward` with an NCCL all-reduce on real-GPU builds, keeping the CPU summation path for CUDA-less test/CI builds.

## Non-Goals
- **No new model architectures and no WIT schema changes.** The `hardware-strategy` / `parallel-execution-plan` records already exist; this change only consumes them. Llama-family TP/PP and Mixtral-style MoE remain the supported set, exactly as the engines were built.
- **No KV-cache-in-decode for pipeline stages, no real wall-clock stage overlap** (per-stage threads/processes) — the pipeline path is wired in its current prefill-only form; decode-time per-stage KV cache and threaded overlap are tracked as P1 follow-ups, not this change.
- **No constrained decoding and no NPU/TPU** — those remain their own proposals (`constrained-decoding-activation`, `npu-tpu-real-device-execution`).
- **No ONNX/NVFP4 forward-pass execution** — that is `gpu-accelerated-inference-execution`'s scope. This change only activates the *candle* CUDA build that both depend on, and the multi-GPU enumeration / NCCL all-reduce the parallel engines need.

## Impact
- **Affected capability**: `ai-inference` (delta below). This is the requirement that the parallel engines, once selectable, are actually selected by a deployment and run on real accelerators.
- **Affected code**:
  - `core-host/src/host_core/domain_types.rs` — `IntegrityModelBinding` gains the strategy fields.
  - `core-host/src/ai_inference/candle_llm_runtime.rs` — `LoadedModel::Parallel`, dispatch in `try_load`, generation routing.
  - `core-host/src/ai_inference.rs` — pass the binding's strategy through the `try_load` call site (`~line 951`).
  - `core-host/src/ai_inference/parallel.rs` — NVML enumeration in `discover_cluster_topology()`, NCCL all-reduce in `RowParallelLinear::forward` under the CUDA feature.
  - `core-host/Cargo.toml` — `nvfp4-cuda` pulls `candle-cuda`; add NVML dep behind the CUDA feature.
  - `systems/system-faas-config-api` — surface the strategy fields when constructing the binding (mapping already-validated WIT records).
- **Risk**: the dispatch path is new control flow at model-load time. Mitigated by (a) the `single`/empty-strategy default preserving the untouched dense path, (b) fail-fast topology validation before any weights load, and (c) the CUDA-feature gate keeping CPU/CI builds on the proven `Device::Cpu` summation path so no test behaviour changes without real hardware. The NCCL all-reduce is the highest-risk item and is only reachable on a real multi-GPU build behind a hardware-gated test lane (CI CUDA jobs #196/#197 are the starting point).
