## Context

Tachyon already has Candle ONNX inference, model bindings, batching, layer-wise safetensors mapping, LoRA routing, and GPU-aware scheduling. The missing piece is not model-specific text generation; it is correct handling of NVIDIA TensorRT ModelOpt NVFP4 tensor storage.

ModelOpt/NVFP4 checkpoints use safetensors files where linear operators may contain packed FP4 `weight` tensors, FP8 E4M3 `weight_scale` tensors, scalar `weight_scale_2` tensor-level scales, optional `input_scale` tensors, and BF16/F16 tensors that must pass through untouched. This requires typed tensor layout support and dequant/kernel hooks before any architecture-specific model runtime can consume the weights.

## Goals / Non-Goals

**Goals:**
- Detect ModelOpt/NVFP4 model directories from safetensors index and quantization metadata without requiring a specific model architecture.
- Map ModelOpt tensor groups into typed components for each linear operator.
- Provide a pure Rust NVFP4 dequantization oracle for BF16/F32 fallback and tests.
- Estimate fallback memory for eager and layer-window dequantization and reject over-budget fallback.
- Define native-kernel capability hooks and a feature-gated CUDA/CUTLASS backend for Blackwell FP4 dequant/matmul support.
- Prevent detected NVFP4 bindings from returning mock inference output.

**Non-Goals:**
- Implementing architecture-specific causal-LLM execution, MoE routing, hybrid attention, tokenizer loading, chat templates, prefill/decode, or text generation.
- Supporting every ModelOpt quantization variant beyond the W4A16_NVFP4 layout currently represented by packed FP4 plus FP8 block scales.
- Training, fine-tuning, or re-quantization.
- Guaranteeing native FP4 acceleration on non-Blackwell GPUs.

## Decisions

### Use a generic ModelOpt/NVFP4 component loader

The loader detects NVFP4 through `config.json`, `hf_quant_config.json`, or `.weight_scale_2` tensor names in `model.safetensors.index.json`. It does not validate `architectures`, `model_type`, tokenizer assets, or model-specific fields.

Architecture execution remains a separate future change. This keeps the current work focused on the quantized storage format and avoids coupling dequant/kernel support to one checkpoint family.

### ModelOpt tensors become typed components

Packed FP4 data, FP8 block scales, tensor-level scales, input activation scales, and BF16/F16 tensors are represented explicitly. The loader must never reinterpret packed FP4 payloads as plain `f32`.

The component lookup is driven by tensor names in `model.safetensors.index.json` and safetensors headers. Shard boundaries are not assumed to match layers or operators.

### Provide a pure Rust dequantization oracle before native kernels

The first implementation includes deterministic FP4 nibble unpacking, FP8 E4M3 scale conversion, tensor-level scaling, block-size validation, and BF16/F32 output conversion. Native kernels can later be tested against this oracle.

### Native FP4 execution is capability-gated

Native NVFP4 dequant/matmul kernels should be selected only when the accelerator reports compatible hardware, runtime, and compiled kernel support. Unsupported devices use the dequant fallback when memory limits allow, or fail with a typed error when fallback is not acceptable.

### CUDA/CUTLASS native kernels are feature-gated

The concrete native backend lives behind `nvfp4-cuda`. Standard builds remain pure Rust and do not require CUDA, NVCC, or CUTLASS headers. Enabling `nvfp4-cuda` requires an explicit CUDA toolkit path and CUTLASS include path so the build can compile the native FP4 source and link CUDA runtime libraries.

### Detected NVFP4 must not use mock output

For non-NVFP4 model bindings, existing ONNX/mock behavior is preserved. For detected ModelOpt/NVFP4 directories, the runtime registers the typed component set and returns an explicit unsupported-execution error until a concrete architecture runtime or kernel backend is wired.

## Risks / Trade-offs

- ModelOpt layouts may vary across NVIDIA releases. Mitigation: detect the specific W4A16_NVFP4 layout and reject unknown group sizes/components with structured errors.
- Eager dequant fallback can exceed memory budgets. Mitigation: estimate packed plus dense memory and support layer-window estimates.
- Native FP4 kernel availability may lag hardware variants. Mitigation: keep native kernels behind capability checks and preserve fallback behavior.
- Avoiding architecture support means the model will not generate text yet. This is intentional for the current scope.

## Migration Plan

1. Add generic ModelOpt/NVFP4 detection and typed load errors.
2. Add safetensors index/header parsing and typed linear component mapping.
3. Add pure Rust dequant tests and memory-limit checks.
4. Wire detected NVFP4 bindings into the AI runtime as non-mock component sets.
5. Add native-kernel capability selection hooks and CUDA/CUTLASS FFI.
6. Extend layer-wise mapping to stream packed components by tensor name.
7. Add docs and gated real-checkpoint probes.

Rollback is simple: disable the ModelOpt/NVFP4 detector and retain ONNX/mock behavior for existing bindings.

## Open Questions

- Should fallback dequant support BF16 only for production memory profiles, or keep F32 available for test/reference use?
- Which real ModelOpt/NVFP4 checkpoint should be used for a gated loading probe independent of architecture execution?
