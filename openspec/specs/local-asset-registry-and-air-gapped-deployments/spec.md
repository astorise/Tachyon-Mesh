# local-asset-registry-and-air-gapped-deployments Specification

## Purpose
TBD - created by archiving change local-asset-registry-and-air-gapped-deployments. Update Purpose after archive.
## Requirements
### Requirement: The mesh stores deployable binaries in an embedded asset registry
The host SHALL persist deployable binaries in an embedded registry keyed by SHA-256 hashes.

#### Scenario: A new asset is uploaded
- **WHEN** an administrator uploads a `.wasm` binary
- **THEN** the host stores it under `sha256:<digest>` in persistent storage
- **AND** it returns the canonical `tachyon://sha256:<digest>` URI

### Requirement: Asset uploads are protected by admin authentication
Asset upload endpoints SHALL reuse the admin authentication middleware before accepting new binaries.

#### Scenario: An unauthenticated caller uploads an asset
- **WHEN** a request reaches the asset upload endpoint without a valid admin token
- **THEN** the host rejects the request

### Requirement: Wasmtime can resolve local registry URIs
The runtime SHALL resolve `tachyon://sha256:...` module references from the embedded registry before instantiating them.

#### Scenario: A manifest references an embedded asset URI
- **WHEN** a route or system module uses `tachyon://sha256:<digest>` as its module reference
- **THEN** the host loads the binary from the embedded asset registry
- **AND** it provides the resulting bytes to the normal Wasmtime load path without contacting any external registry

