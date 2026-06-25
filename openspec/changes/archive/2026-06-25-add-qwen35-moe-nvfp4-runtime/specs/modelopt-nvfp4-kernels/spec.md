## ADDED Requirements

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
