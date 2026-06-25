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

The runtime SHALL execute packed NVFP4 weights through native kernels only when compatible hardware, drivers, and compiled kernels are available; otherwise it SHALL use the bounded fallback or reject execution.

#### Scenario: Native kernels are available

- **WHEN** a compatible CUDA backend and native NVFP4 kernels are available
- **THEN** inference executes without eager full-model dense dequantization
- **AND** output is validated against the fallback reference

#### Scenario: Native kernels are unavailable

- **WHEN** native execution is unavailable
- **THEN** the runtime uses the bounded fallback only when configured memory limits permit it
- **AND** otherwise returns a typed unsupported-execution error

### Requirement: CUDA/CUTLASS NVFP4 backend MUST be feature-gated
The runtime SHALL expose a concrete CUDA/CUTLASS native backend only when the `nvfp4-cuda` feature is enabled and the build has CUDA/CUTLASS inputs.

#### Scenario: Standard build does not require CUDA
- **WHEN** `nvfp4-cuda` is not enabled
- **THEN** Tachyon builds and tests without CUDA, NVCC, or CUTLASS headers
- **AND** native NVFP4 capability is reported unavailable

#### Scenario: Feature-only build does not require CUDA inputs
- **WHEN** `nvfp4-cuda` is enabled without CUDA toolkit or CUTLASS include paths
- **THEN** Tachyon builds the Rust NVFP4 capability layer without compiling native CUDA kernels
- **AND** native NVFP4 capability is reported unavailable

#### Scenario: CUDA/CUTLASS build exposes native kernels
- **WHEN** `nvfp4-cuda` is enabled with CUDA toolkit and CUTLASS include paths
- **THEN** the build compiles the native NVFP4 CUDA source
- **AND** runtime capability checks can report compiled dequant and matmul kernels

### Requirement: Mixed FP8 and NVFP4 operators MUST compose in one forward graph

The ModelOpt runtime SHALL execute a single architecture graph containing dense
BF16/F16 tensors, FP8 projections, and W4A16 NVFP4 projections according to the
checkpoint's quantized-layer metadata.

#### Scenario: Operator selects declared quantization

- **WHEN** quantization metadata assigns FP8, W4A16 NVFP4, or dense storage to an
  operator
- **THEN** the runtime dispatches that operator through the matching typed
  implementation

#### Scenario: Unknown mixed-precision assignment is rejected

- **WHEN** quantization metadata assigns an unsupported algorithm to a required
  operator
- **THEN** loading fails with the operator name and quantization algorithm

### Requirement: NVFP4 sparse experts MUST avoid full-model densification

The runtime SHALL execute selected NVFP4 experts without eagerly dequantizing
all experts or all layers into dense accelerator tensors.

#### Scenario: Only active experts are materialized

- **WHEN** sparse routing selects a subset of experts
- **THEN** only those experts and shared experts are transferred, dequantized,
  or executed for that token batch

#### Scenario: Fallback is memory bounded

- **WHEN** native NVFP4 expert kernels are unavailable
- **THEN** fallback dequantization is limited to the active expert/layer window
- **AND** execution is rejected when configured host or accelerator memory
  limits would be exceeded

### Requirement: Native mixed-precision execution MUST remain capability-gated

Production-sized compatible checkpoints SHALL use native FP8/NVFP4 kernels only
when hardware, runtime, and compiled-kernel capabilities satisfy the operator
requirements.

#### Scenario: Production checkpoint lacks required native capability

- **WHEN** a checkpoint exceeds the configured fallback memory threshold
- **AND** required native FP8 or NVFP4 kernels are unavailable
- **THEN** the runtime rejects execution with the missing capabilities

#### Scenario: Compatible native backend executes packed weights

- **WHEN** the accelerator reports all required native capabilities
- **THEN** packed FP8/NVFP4 weights execute without full eager dense conversion

### Requirement: NVFP4 execution path MUST be observable

The runtime SHALL record whether each NVFP4 request used native FP4, dense GPU fallback, CPU fallback, or failed before execution.

#### Scenario: Operator inspects an NVFP4 request

- **WHEN** an operator reads inference telemetry
- **THEN** the actual execution path is present without requiring source inspection

