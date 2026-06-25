## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: NVFP4 execution path MUST be observable

The runtime SHALL record whether each NVFP4 request used native FP4, dense GPU fallback, CPU fallback, or failed before execution.

#### Scenario: Operator inspects an NVFP4 request

- **WHEN** an operator reads inference telemetry
- **THEN** the actual execution path is present without requiring source inspection
