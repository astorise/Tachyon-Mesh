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

### Requirement: Node artifacts MUST be distributed via PUSH from the Control Plane
The Tachyon Mesh nodes SHALL NOT pull execution artifacts (Wasm components, models) from external public networks. Artifacts MUST be pushed to the internal mesh storage broker by a trusted client (UI/MCP) to guarantee Air-Gap compliance.

#### Scenario: Deploying a new Wasm function to an Air-Gapped factory
- **GIVEN** an edge node with no outbound internet access
- **WHEN** the operator uploads a `.wasm` file via Tachyon-UI
- **THEN** the UI pushes the binary payload to the internal mesh storage
- **AND** updates the GitOps configuration with the `sha256` hash of the asset
- **AND** the `core-host` successfully verifies the hash against the local file and executes it without network calls.

### Requirement: Execution integrity MUST rely on cryptographic signatures
The `core-host` SHALL refuse to instantiate any Wasm component or load any AI model whose physical file hash does not strictly match the `sha256` declared in the GitOps `AssetBundle`, mitigating any internal storage tampering.

#### Scenario: Rejecting a tampered pushed artifact
- **GIVEN** an artifact bundle declares a `sha256` and Ed25519 signature
- **WHEN** the local asset registry contains bytes that do not match the declared hash
- **THEN** the host refuses to load the artifact
- **AND** emits a security event identifying the failed integrity check.

