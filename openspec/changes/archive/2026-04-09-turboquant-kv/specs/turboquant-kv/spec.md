## ADDED Requirements

### Requirement: Candle can invoke TurboQuant through a native FFI bridge
The runtime SHALL expose the TurboQuant compression and decompression kernels to Rust through a dedicated FFI layer that can be called safely from Candle custom operators.

#### Scenario: The build enables TurboQuant support
- **WHEN** the workspace builds with TurboQuant integration enabled
- **THEN** the native C++ or CUDA sources are compiled through a Rust FFI crate
- **AND** the Rust runtime can call the exported TurboQuant entrypoints from Candle code

### Requirement: TurboQuant applies asymmetric KV compression
The inference runtime SHALL preserve the `K` cache in standard precision and SHALL only apply TurboQuant compression to the `V` cache.

#### Scenario: An attention layer stores a KV cache entry
- **WHEN** the model writes cache state for a TurboQuant-enabled layer
- **THEN** the `K` tensors remain in `q8_0` or `f16`
- **AND** only the `V` tensors are passed through the TurboQuant compression path

### Requirement: Boundary layers bypass TurboQuant value compression
The inference runtime SHALL protect the first two and last two transformer layers from TurboQuant value compression.

#### Scenario: The model evaluates a protected boundary layer
- **WHEN** the active layer index is within the first two or last two transformer blocks
- **THEN** the runtime bypasses TurboQuant for that layer's `V` cache
- **AND** keeps the cache in the standard high-precision representation

### Requirement: TurboQuant supports sparse value decoding from attention scores
The TurboQuant decompression path SHALL accept attention weights and a threshold so near-zero attention entries can skip value decoding work.

#### Scenario: A cached token has negligible attention weight
- **WHEN** the decompression path sees an attention weight below the configured threshold
- **THEN** it skips the value decode work for that token
- **AND** returns the equivalent zero contribution for the skipped entry

### Requirement: TurboQuant integration is validated against reference fixtures
The TurboQuant integration SHALL be verified against reference fixtures generated from the native implementation before it is used in model integration.

#### Scenario: The fixture validation test runs
- **WHEN** the Rust validation suite loads the reference TurboQuant fixtures
- **THEN** the Candle custom operator output matches the reference packed representation bit-for-bit
- **AND** the implementation is considered ready for model-level integration
