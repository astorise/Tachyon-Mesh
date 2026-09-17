# turboquant-kv Specification

## Purpose
Define the TurboQuant KV cache integration contract for Candle-backed inference.
## Requirements
### Requirement: Candle can invoke TurboQuant through a native FFI bridge
The runtime SHALL expose the TurboQuant compression and decompression kernels to Rust through a
dedicated FFI layer that can be called safely from Candle custom operators.

#### Scenario: The build enables TurboQuant support
- **WHEN** the workspace builds with TurboQuant integration enabled
- **THEN** the native C++ or CUDA sources are compiled through a Rust FFI crate
- **AND** the Rust runtime can call the exported TurboQuant entrypoints from Candle code

### Requirement: TurboQuant applies asymmetric KV compression
The inference runtime SHALL preserve the `K` cache in standard precision and SHALL only apply
TurboQuant compression to the `V` cache.

#### Scenario: An attention layer stores a KV cache entry
- **WHEN** the model writes cache state for a TurboQuant-enabled layer
- **THEN** the `K` tensors remain in `q8_0` or `f16`
- **AND** only the `V` tensors are passed through the TurboQuant compression path

### Requirement: Boundary layers bypass TurboQuant value compression
The inference runtime SHALL protect the first two and last two transformer layers from TurboQuant
value compression.

#### Scenario: The model evaluates a protected boundary layer
- **WHEN** the active layer index is within the first two or last two transformer blocks
- **THEN** the runtime bypasses TurboQuant for that layer's `V` cache
- **AND** keeps the cache in the standard high-precision representation

### Requirement: TurboQuant supports sparse value decoding from attention scores
The TurboQuant decompression path SHALL accept attention weights and a threshold so near-zero
attention entries can skip value decoding work.

#### Scenario: A cached token has negligible attention weight
- **WHEN** the decompression path sees an attention weight below the configured threshold
- **THEN** it skips the value decode work for that token
- **AND** returns the equivalent zero contribution for the skipped entry

### Requirement: TurboQuant integration is validated against reference fixtures
The TurboQuant integration SHALL be verified against reference fixtures generated from the native
implementation before it is used in model integration.

#### Scenario: The fixture validation test runs
- **WHEN** the Rust validation suite loads the reference TurboQuant fixtures
- **THEN** the Candle custom operator output matches the reference packed representation bit-for-bit
- **AND** the implementation is considered ready for model-level integration

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

### Requirement: Distributed AI KV Caches MUST be isolated by Tenant
When the `system-faas-model-broker` stores context tensors in Turboquant, the storage layer SHALL enforce logical isolation based on the `tenant_id` extracted from the active request context. A cache hit MUST NOT occur across different tenant boundaries, preventing prompt-bleeding vulnerabilities.

#### Scenario: Tenant A and Tenant B using the same model
- **GIVEN** a shared model and a distributed KV cache with `tenant_isolation: true`
- **WHEN** Tenant B submits a prompt identical to Tenant A's previous prompt
- **THEN** the cache engine treats it as a cache miss and recomputes the context, ensuring zero cross-tenant data exposure.

### Requirement: Gossiped Cache State MUST utilize Transparent Data Encryption
When configured, the cache synchronization protocol SHALL encrypt all KV cache tensors in transit and at rest using the cluster's TDE keys, ensuring that physical access to the Edge node or interception of the network overlay does not expose user inference data.

#### Scenario: Encrypting distributed KV cache replication
- **GIVEN** a distributed KV cache configured with transparent data encryption
- **WHEN** cache tensors are replicated to a peer node
- **THEN** the synchronization payload is encrypted with the active cluster TDE key
- **AND** persisted replicas remain encrypted at rest.

### Requirement: KV Partitions MUST be provisioned dynamically
The underlying Turboquant embedded database SHALL allocate isolated partitions (namespaces) based on the `kv_partitions` list in the declarative configuration.

#### Scenario: Backing up a KV partition to S3
- **GIVEN** an active S3 backend named `corporate-blob-store`
- **WHEN** a KV partition is configured with `sync_to_s3_backend_ref` pointing to that backend
- **THEN** the storage broker begins asynchronously writing the partition's SSTable snapshots to the configured S3 bucket using `system-faas-s3-proxy`.

