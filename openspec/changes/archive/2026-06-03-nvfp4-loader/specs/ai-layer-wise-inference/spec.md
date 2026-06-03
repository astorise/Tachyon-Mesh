## ADDED Requirements

### Requirement: Layer-wise streaming MUST preserve NVFP4 tensor structure
Layer-wise streaming for ModelOpt/NVFP4 checkpoints SHALL map safetensors shards by tensor name and vend typed per-layer quantized components instead of partitioning raw bytes into equal `f32` slices.

#### Scenario: Active layer loads typed NVFP4 components
- **WHEN** `memory-profile` is `layer-wise-streaming`
- **AND** the active layer contains ModelOpt/NVFP4 linear operators
- **THEN** the loader maps the packed weights, block scales, tensor scales, and any BF16 tensors required for that layer
- **AND** it transfers or dequantizes only the active layer's required components according to the selected backend

#### Scenario: Sharded tensor index drives layer mapping
- **WHEN** a ModelOpt/NVFP4 checkpoint uses multiple safetensors shards
- **THEN** the layer-wise loader resolves each tensor through `model.safetensors.index.json`
- **AND** it never assumes all weights for a layer are contiguous in a single equal-sized byte range

### Requirement: Layer-wise NVFP4 execution MUST keep memory-profile semantics
The ModelOpt/NVFP4 layer-wise runtime SHALL preserve the existing performance and layer-wise-streaming memory profile behavior while accounting for packed quantized storage and fallback dequantization.

#### Scenario: Layer-wise streaming avoids full packed-model residency on accelerator
- **WHEN** a ModelOpt/NVFP4 model runs with `layer-wise-streaming`
- **THEN** the runtime does not load all model layers into accelerator memory at once
- **AND** it pages KV cache and layer weights according to the existing layer-wise streaming contract

#### Scenario: Fallback dequantization respects memory limits
- **WHEN** native NVFP4 kernels are unavailable under `layer-wise-streaming`
- **THEN** the runtime may dequantize only the active layer or configured layer window
- **AND** it rejects execution if fallback dequantization would require full-model accelerator residency
