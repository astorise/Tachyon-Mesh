# Proposal: Real GPU Execution for ONNX and NVFP4 Inference Paths

## Why
Two execution paths in the codebase load real weights onto an accelerator but never actually compute on it:

1. **ONNX / WASI-NN (`ai-inference`)**: `openspec/specs/ai-inference/spec.md` already documents — as current, intended behavior — *"Inference executes on CPU; GPU execution is deferred pending upstream candle fix (issue #3491)"*. `candle-onnx`'s `simple_eval` only runs on `Device::Cpu` today because of that upstream limitation. A model binding can declare a GPU `device` in the integrity manifest, but ONNX inference silently still runs on CPU.
2. **ModelOpt/NVFP4 (`modelopt-nvfp4-kernels`)**: the loader (`modelopt_nvfp4.rs` + CUDA kernels) correctly detects, validates, and represents NVFP4 checkpoints, and the spec explicitly requires the runtime to refuse mock output for these aliases. By design, it currently returns an "unsupported-execution error" for every NVFP4 alias — the native CUDA/CUTLASS kernels referenced by `modelopt-nvfp4-kernels`'s "CUDA/CUTLASS NVFP4 backend MUST be feature-gated" requirement are compiled under `nvfp4-cuda`, but no forward pass wires `Tensor` ops through them; the capability layer exists, the matmul does not run.

This means: declaring a GPU device for an AI target, or deploying an NVFP4-quantized model, today buys no GPU acceleration. This is the second-highest priority gap after model-parallelism, and blocks any throughput/latency-sensitive deployment.

## What Changes
1. **ONNX GPU execution**: track and adopt the upstream candle fix for issue #3491 (or implement an interim workaround — e.g., a CUDA-backed `candle-onnx` op dispatch table) so `CandleOnnxGraph::compute` executes on the device declared by the model binding instead of being hardcoded to CPU.
2. **NVFP4 native forward pass**: implement the actual dequant + matmul forward pass against the compiled CUDA/CUTLASS kernels (`nvfp4-cuda` feature) so a model classified as ModelOpt/NVFP4 with native FP4 capability available executes real inference instead of returning the unsupported-execution error.
3. **Capability-gated fallback**: when native FP4 kernels are unavailable but the BF16/F32 fallback dequantization (already specified) fits within configured memory limits, execute inference via that fallback path on GPU, rather than rejecting outright. (`modelopt-nvfp4-kernels`'s existing "Unsupported accelerator falls back or rejects" requirement already describes this; this proposal is what makes "falls back" possible instead of always "rejects".)
4. **Telemetry**: surface which device class (CPU/GPU-native-FP4/GPU-fallback) actually executed each inference call, so operators can distinguish "ran on GPU" from "silently ran on CPU" — directly observable today only by reading source code.

## Non-Goals
- Does not add new quantization formats beyond NVFP4.
- Does not implement multi-GPU sharding for these single-device paths (see the separate model-parallelism proposal).
- Does not change the NVFP4 loader/detection logic (`modelopt-nvfp4-kernels` load-time requirements are unaffected); only the execution-time gap is addressed.

## Impact
- **Affected capabilities**: `ai-inference` (ONNX device routing), `modelopt-nvfp4-kernels` (forward pass).
- **Affected code**: `core-host` ONNX/WASI-NN bridge, NVFP4 CUDA kernel bindings, `system-faas-model-broker` device selection.
- **Risk**: upstream candle issue #3491 may not be fixable purely downstream; if so, the ONNX portion of this change is scoped to an interim workaround with an explicit removal task once upstream lands. The NVFP4 portion has no such external blocker — the kernels are already vendored.
