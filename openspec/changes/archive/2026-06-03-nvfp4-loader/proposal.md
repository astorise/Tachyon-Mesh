## Why

Tachyon can route model artifacts through Candle-backed inference, but it does not yet understand NVIDIA TensorRT ModelOpt NVFP4 storage. ModelOpt/NVFP4 is not a normal safetensors float layout: packed FP4 weights, FP8 E4M3 block scales, tensor-level scales, and BF16/F16 passthrough tensors must be represented and dequantized explicitly. Treating these bytes as plain `f32` would produce invalid tensors.

This change is scoped to NVFP4 support primitives: detection, safetensors component mapping, dequantization fallback, memory estimates, and native-kernel capability hooks. It does not add architecture-specific causal-LLM execution, tokenizer handling, MoE routing, or text generation.

## What Changes

- Add generic detection for Hugging Face-style ModelOpt/NVFP4 model directories.
- Parse `model.safetensors.index.json` and optional ModelOpt metadata without gating on a specific architecture.
- Represent packed FP4 weights, FP8 E4M3 block scales, tensor-level scales, activation input scales, and BF16/F16 passthrough tensors as typed components.
- Add deterministic pure-Rust NVFP4 dequantization into BF16/F32 for correctness tests and unsupported accelerators.
- Add fallback memory estimates and limit checks for eager and layer-window dequantization.
- Add accelerator capability hooks and a feature-gated CUDA/CUTLASS native NVFP4 backend.
- Preserve existing ONNX and mock model behavior for non-NVFP4 bindings.
- Return explicit unsupported-execution errors when a ModelOpt/NVFP4 directory is detected but no architecture runtime/native kernel path is configured.

## Capabilities

### New Capabilities
- `modelopt-nvfp4-kernels`: Defines the contract for loading ModelOpt/NVFP4 tensor components, dequantizing them, and selecting native/fallback execution primitives.

### Modified Capabilities
- `ai-inference`: Detects ModelOpt/NVFP4 bindings and prevents them from falling through to mock inference output.
- `ai-layer-wise-inference`: Extends layer-wise mapping requirements so packed NVFP4 shards and scale tensors are resolved by tensor name instead of raw byte partitions.

## Impact

- Affected code: `core-host/src/ai_inference.rs`, `core-host/src/ai_inference/modelopt_nvfp4.rs`, CUDA/CUTLASS FFI sources, OpenSpec AI inference artifacts, and AI inference tests.
- No tokenizer dependency is required for this scope.
- Runtime behavior changes only for detected ModelOpt/NVFP4 directories: they are classified as NVFP4 component sets and fail with an actionable unsupported-execution message until a concrete execution backend is wired.
- Hardware behavior becomes capability-aware: native FP4 kernels can be selected only when the host reports compatible hardware/runtime/kernel support; otherwise dequantized fallback is used or rejected by memory limits.
