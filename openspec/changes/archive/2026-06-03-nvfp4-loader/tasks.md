## 1. ModelOpt/NVFP4 Detection and Validation

- [x] 1.1 Add a generic ModelOpt/NVFP4 detector in the AI model binding load path.
- [x] 1.2 Parse optional `config.json`, optional `hf_quant_config.json`, and required `model.safetensors.index.json` without architecture-specific validation.
- [x] 1.3 Verify every shard referenced by `model.safetensors.index.json` exists before registering the model alias.
- [x] 1.4 Add typed load errors for missing indexes, missing shards, invalid JSON, invalid safetensors headers, unsupported quantization layouts, and missing tensor components.
- [x] 1.5 Add synthetic tests for valid NVFP4 detection, non-NVFP4 directories, missing indexes, missing shards, and unsupported group sizes.

## 2. NVFP4 Tensor Representation

- [x] 2.1 Add typed structs for packed FP4 weights, FP8 E4M3 block scales, tensor-level scales, activation input scales, and BF16/F16 passthrough tensors.
- [x] 2.2 Implement safetensors lookup helpers that resolve named tensors through the shard index without assuming contiguous layer byte ranges.
- [x] 2.3 Map ModelOpt tensor groups such as `weight`, `weight_scale`, `weight_scale_2`, and `input_scale` into per-linear operator components.
- [x] 2.4 Ensure packed FP4 tensors are never converted with `f32::from_le_bytes` in the ModelOpt/NVFP4 path.
- [x] 2.5 Add synthetic safetensors fixtures that cover packed weights, scales, BF16 tensors, missing components, and malformed shapes.

## 3. Dequantization Fallback

- [x] 3.1 Implement a pure Rust NVFP4 dequantization module inspired by `mold-ai-inference::nvfp4`.
- [x] 3.2 Support FP8 E4M3 scale conversion, FP4 nibble unpacking, block-size validation, tensor-level scaling, and BF16/F32 output selection.
- [x] 3.3 Add deterministic fixture tests for nibble order, block scale ordering, tensor scale application, NaN/zero scale handling, and invalid K dimensions.
- [x] 3.4 Add memory estimation for eager and layer-window dequantization fallback.
- [x] 3.5 Reject fallback execution when estimated host RAM or accelerator memory exceeds configured limits.

## 4. Runtime Integration Boundary

- [x] 4.1 Remove architecture-specific tokenizer/text-generation scope from the implementation and dependency graph.
- [x] 4.2 Register detected ModelOpt/NVFP4 directories as typed non-mock backend entries.
- [x] 4.3 Return an explicit unsupported-execution error for detected NVFP4 aliases until native kernels or an architecture runtime are configured.
- [x] 4.4 Preserve existing Candle ONNX and mock behavior for non-NVFP4 bindings.

## 5. Native NVFP4 Kernel Hooks

- [x] 5.1 Define an accelerator capability interface for native NVFP4 dequant and matmul kernels.
- [x] 5.2 Gate native FP4 kernel selection on compatible hardware, runtime availability, and compiled kernel support.
- [x] 5.3 Add a placeholder or feature-gated backend plan for native FP4 execution without changing fallback semantics.
- [x] 5.4 Add tests that verify unsupported accelerators choose fallback or reject execution according to memory limits.
- [x] 5.5 Document minimum hardware/runtime requirements for enabling native NVFP4 acceleration.

## 6. Layer-Wise Streaming Integration

- [x] 6.1 Replace equal-byte layer partitioning for ModelOpt/NVFP4 with tensor-name-driven layer mapping from the safetensors index.
- [x] 6.2 Implement active-layer loading for packed weights, scales, BF16/F16 tensors, and fallback dense windows.
- [x] 6.3 Support dequantizing only the active layer or configured layer window when native NVFP4 kernels are unavailable.
- [x] 6.4 Preserve the existing `performance` and `layer-wise-streaming` memory profile semantics.
- [x] 6.5 Add tests showing sharded tensors resolve correctly and layer-wise streaming does not require full-model accelerator residency.

## 7. Reference and Real-Checkpoint Validation

- [x] 7.1 Add a gated probe test controlled by an environment variable pointing at a local ModelOpt/NVFP4 checkpoint directory.
- [x] 7.2 Validate real-checkpoint loading, shard resolution, and quantized component mapping without requiring CI to download the model.
- [x] 7.3 Compare fallback dequant outputs or kernel outputs against a reference runtime/module for fixed synthetic tensors.
- [x] 7.4 Update troubleshooting or AI inference docs with supported ModelOpt/NVFP4 layout, unsupported variants, fallback limits, and expected errors.

## 8. Concrete CUDA/CUTLASS Native Backend

- [x] 8.1 Add an `nvfp4-cuda` feature and build script support for explicit CUDA toolkit and CUTLASS include discovery.
- [x] 8.2 Add concrete native CUDA/CUTLASS sources for NVFP4 dequantization and native matmul entrypoints.
- [x] 8.3 Expose a Rust FFI backend that reports native capability and can launch the CUDA dequant/matmul entrypoints.
- [x] 8.4 Add compile-safe Rust tests plus gated native smoke tests that run only when CUDA/CUTLASS is configured.
- [x] 8.5 Update docs/specs with the concrete `nvfp4-cuda` build contract and runtime boundary.
