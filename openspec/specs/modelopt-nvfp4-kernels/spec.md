# modelopt-nvfp4-kernels Specification

## Purpose
TBD - created by archiving change nvfp4-loader. Update Purpose after archive.
## Requirements
### Requirement: ModelOpt/NVFP4 directories MUST be detected and validated
The host SHALL detect TensorRT ModelOpt/NVFP4 Hugging Face-style model directories from quantization metadata or safetensors tensor names without requiring a specific model architecture.

#### Scenario: Valid ModelOpt/NVFP4 directory is accepted
- **WHEN** a model binding points at a directory containing `model.safetensors.index.json` and all referenced `.safetensors` shards
- **AND** either metadata declares `W4A16_NVFP4` or indexed tensors include ModelOpt NVFP4 scale components
- **THEN** the host classifies the binding as a ModelOpt/NVFP4 component set
- **AND** records the tensor shard map and quantization layout

#### Scenario: Missing shard is rejected
- **WHEN** `model.safetensors.index.json` references a shard that is absent on disk
- **THEN** model loading fails before inference starts
- **AND** the error identifies the missing shard and model alias

#### Scenario: NVFP4 metadata without safetensors index is rejected
- **WHEN** `config.json` or `hf_quant_config.json` declares NVFP4 quantization
- **AND** `model.safetensors.index.json` is missing
- **THEN** model loading fails with a missing-index error

### Requirement: ModelOpt NVFP4 tensors MUST be represented as typed quantized components
The loader SHALL represent packed FP4 weights, FP8 E4M3 block scales, tensor-level scales, activation input scales, and BF16/F16-sensitive tensors as distinct typed components.

#### Scenario: Packed weights are not interpreted as f32 payloads
- **WHEN** the loader reads a safetensors entry for a ModelOpt NVFP4 linear operator
- **THEN** it stores packed weight bytes as packed FP4 data
- **AND** it associates `weight_scale`, `weight_scale_2`, and optional `input_scale` tensors according to the tensor name map
- **AND** it does not convert packed bytes with `f32::from_le_bytes`

#### Scenario: Sensitive BF16/F16 tensors remain unquantized
- **WHEN** the loader reads tensors declared as BF16 or F16
- **THEN** it preserves their declared dtype
- **AND** it does not require FP4 scale tensors for those weights

### Requirement: NVFP4 dequantization MUST have a correctness-first fallback
The runtime SHALL provide deterministic fallback dequantization from packed NVFP4 weights and FP8 block scales into BF16 or F32 tensors.

#### Scenario: Synthetic NVFP4 fixture dequantizes deterministically
- **WHEN** a test fixture provides packed FP4 values, FP8 E4M3 block scales, and a tensor-level scale
- **THEN** the fallback dequantizer returns the expected dense values
- **AND** validates nibble order, block size, scale shape, and tensor-level scaling

#### Scenario: Invalid block layout is rejected
- **WHEN** a packed NVFP4 tensor has a K dimension incompatible with the supported block size
- **THEN** dequantization fails with a typed shape/layout error
- **AND** no partial dense tensor is returned

### Requirement: Native FP4 acceleration MUST be capability-gated
The runtime SHALL only select native NVFP4 dequant/matmul kernels when the accelerator backend reports compatible hardware, driver/runtime support, and kernel availability.

#### Scenario: Compatible backend selects native FP4
- **WHEN** the selected accelerator reports native FP4 capability
- **AND** required NVFP4 kernels are compiled and available
- **THEN** the runtime may execute packed FP4 weights without eager BF16/F32 dequantization

#### Scenario: Unsupported accelerator falls back or rejects
- **WHEN** the selected accelerator lacks native NVFP4 support
- **AND** fallback dequantization is allowed within configured memory limits
- **THEN** the runtime uses the BF16/F32 fallback path
- **AND** if fallback exceeds configured limits, startup or inference fails with an unsupported-accelerator error

### Requirement: CUDA/CUTLASS NVFP4 backend MUST be feature-gated
The runtime SHALL expose a concrete CUDA/CUTLASS native backend only when the `nvfp4-cuda` feature is enabled and the build has CUDA/CUTLASS inputs.

#### Scenario: Standard build does not require CUDA
- **WHEN** `nvfp4-cuda` is not enabled
- **THEN** Tachyon builds and tests without CUDA, NVCC, or CUTLASS headers
- **AND** native NVFP4 capability is reported unavailable

#### Scenario: CUDA/CUTLASS build exposes native kernels
- **WHEN** `nvfp4-cuda` is enabled with CUDA toolkit and CUTLASS include paths
- **THEN** the build compiles the native NVFP4 CUDA source
- **AND** runtime capability checks can report compiled dequant and matmul kernels
