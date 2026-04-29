## ADDED Requirements

### Requirement: TurboQuant quantization is provided by a pure Rust dependency
The Mesh SHALL implement TurboQuant KV-cache quantization (PolarQuant and QJL) using a native Rust crate, replacing the previous `turboquant-sys` C++ FFI bridge entirely.

#### Scenario: Workspace contains no C++ TurboQuant FFI artifacts
- **WHEN** the workspace is built
- **THEN** no crate depends on `turboquant-sys`
- **AND** no `build.rs` in the workspace requires a C++ compiler for TurboQuant
- **AND** the resulting binary is statically linked against a pure-Rust TurboQuant implementation

### Requirement: Quantization callers use a safe Rust API
The `core-host` AI inference path (`core-host/src/ai_inference.rs` or the relevant System FaaS) SHALL invoke TurboQuant exclusively through the safe Rust API, with no `unsafe extern "C"` boundary crossings introduced for quantization.

#### Scenario: PolarQuant compresses a KV cache via the safe API
- **WHEN** the inference path quantizes a KV cache tensor with PolarQuant
- **THEN** the call is made through the native Rust crate's safe API
- **AND** the resulting quantized tensor is bit-equivalent (within the algorithm's documented tolerance) to a baseline reference computed by the same crate
- **AND** the call site contains no `unsafe` block introduced for the quantization itself
