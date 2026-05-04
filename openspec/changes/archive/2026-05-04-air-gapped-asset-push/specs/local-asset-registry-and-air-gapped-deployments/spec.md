# local-asset-registry-and-air-gapped-deployments Delta

## ADDED Requirements

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
